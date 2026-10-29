//! The owner's commands: `wanderbooks serve`, `close`, `books` (list,
//! create, adopt, restore, back-up, upgrade), `token`, `connect`, `doctor`
//! and `passphrase` (plan 3.3, 3.7, 3.9, 5.5), and the same work for the
//! viewer's Books menu (`new_books`, `back_up_now`, `plan_restore`,
//! `carry_out_restore`, `connect_agent`, `close_books`, `hold_for_viewer`,
//! `upgrade_format`).
//!
//! ```text
//! GOAL: from a terminal, or the viewer's menu, the owner holds the books
//!       open, closes them, makes, adopts, backs up and restores books and
//!       connects agents, without ever naming or touching the books'
//!       private folder.
//! SOURCE: plan/served.md 3.9 (a terminal running `wanderbooks serve`, and
//!         any command that needs the server, holds a lease; "Close the
//!         books" ends every lease), 3.1 (the CLI talks to admin.sock and
//!         never reads the books), 5.5 (adopt; preview and confirmation at
//!         the terminal).
//! INVARIANTS:
//!   1. A command that needs the server holds a lease while it runs, and
//!      lets go when it ends.
//!   2. Nothing printed names the data root.
//!   3. Adoption and restoring are previewed first and go ahead only on a
//!      yes at the terminal, or --yes; a vault beside the books must be
//!      named (or --no-vault given): it is never silently left behind.
//!   4. A token made is kept in its token file (tokens.rs), and read back
//!      from it as kept, before anything is said of it, and its text is
//!      printed only by `token print`; one that can't be kept (or read
//!      back) is revoked again. A token revoked is forgotten by the token
//!      file too. `token check` says whether the file's token is the
//!      books' live one, by its SHA-256 alone, never its text. `connect`
//!      makes the books' agent token only when there is no token file for
//!      it, replacing the server's (no one could be using it: the bridge
//!      reads its token from this file).
//!   5. The viewer's hold lets go only when the owner closes the books;
//!      books that stop, or won't open, are tried again.
//!   6. Books waiting for a format upgrade (plan 3.9, D-39): the owner's
//!      commands that open the books (`serve`, `books`, `token`,
//!      `connect`) ask for the go-ahead once each time they run, at a
//!      terminal that shows the question (input and output a terminal, in
//!      the foreground: the question, then the owner passphrase, which only
//!      the server checks); elsewhere they only say so, on stderr. Only a passphrase typed
//!      at a terminal (or in the viewer) is sent, through the lease
//!      socket's consent_format_upgrade; declining changes nothing.
//! ```

use crate::knobs::{FLAG, Knobs};
use crate::lease::{Ask, HostMsg, Intent, Kind, Reason};
use crate::opener::{HoldError, Lease, Opener};
use ledger_mcp::admin::{self, Request, Response};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// How long a command waits for the books' host to answer one question.
const HOST_WAIT: Duration = Duration::from_secs(5);
/// How long a command waits for the books server to list the books.
const LIST_WAIT: Duration = Duration::from_secs(30);
/// How long `close` waits to see the books closed.
const CLOSE_WAIT: Duration = Duration::from_secs(120);

/// How long the owner's go-ahead for a format upgrade may take: the whole
/// books are backed up and proven first.
const UPGRADE_WAIT: Duration = Duration::from_secs(60 * 60);
/// How long a command waits to learn whether books wait for a format
/// upgrade before it gets on with its own work: a server slow to list
/// them (frozen, or busy) delays the command no more than this.
const OFFER_WAIT: Duration = Duration::from_secs(5);

/// Common flags: `--root DIR` and the test knobs.
#[derive(Default, Clone)]
pub struct Common {
    /// The data root named (`--root`), else the one this account's host
    /// finds.
    pub root: Option<PathBuf>,
    /// The test knobs.
    pub knobs: Knobs,
    /// Ask about books waiting for a format upgrade when the books open
    /// (INVARIANT 6): the owner's own commands; never the viewer's work
    /// (its window asks) nor the agent's file commands.
    pub ask_upgrades: bool,
}

/// Whether this process asked about waiting format upgrades already: once
/// each time a command runs (INVARIANT 6).
static ASKED_UPGRADES: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

impl Common {
    /// Take a common flag (`Ok(true)`), or leave the argument.
    fn take(&mut self, arg: &str, args: &mut impl Iterator<Item = String>) -> Result<bool, String> {
        match arg {
            "--root" => {
                self.root = Some(PathBuf::from(args.next().ok_or("--root needs a folder")?));
                Ok(true)
            }
            FLAG => {
                self.knobs = Knobs::parse(&args.next().ok_or("--test-knobs needs a value")?)?;
                Ok(true)
            }
            _ => Ok(false),
        }
    }
}

fn exit(code: i32) -> Box<dyn std::error::Error> {
    Box::new(crate::Exit(code))
}

/// Hold the books and wait until they are open (INVARIANT 1).
async fn open(opener: &mut Opener) -> Result<Lease, HoldError> {
    let deadline = tokio::time::Instant::now() + opener.knobs().open_wait;
    let too_long = || -> HoldError {
        "the books didn't open in time: run `wanderbooks doctor` to see why".into()
    };
    loop {
        let mut lease = tokio::time::timeout_at(deadline, opener.hold())
            .await
            .map_err(|_| too_long())??;
        loop {
            match tokio::time::timeout_at(deadline, lease.next()).await {
                Err(_) => return Err(too_long()),
                Ok(Some(HostMsg::Ready)) => {
                    opener.opened();
                    return Ok(lease);
                }
                Ok(Some(HostMsg::Ending {
                    reason: Reason::ClosedByOwner,
                    ..
                })) => {
                    return Err(HoldError::ClosedByOwner);
                }
                Ok(Some(HostMsg::Ending {
                    reason: Reason::Restarting | Reason::Closing,
                    ..
                })) => break,
                Ok(Some(HostMsg::Ending { .. })) | Ok(None) => {
                    // The wait before the next try counts against the
                    // deadline too.
                    tokio::time::timeout_at(deadline, opener.failed())
                        .await
                        .map_err(|_| too_long())??;
                    break;
                }
                Ok(Some(_)) => {}
            }
        }
    }
}

/// Ask the server's admin socket, holding the books open meanwhile.
fn with_server(common: &Common, request: Request) -> Result<Response, String> {
    with_server_in(common, request).map(|(answer, _)| answer)
}

/// [`with_server`], with the root's `run/` folder too (where the
/// token files live).
fn with_server_in(common: &Common, request: Request) -> Result<(Response, PathBuf), String> {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|e| e.to_string())?;
    let outcome = rt.block_on(async {
        let mut opener = Opener::new(common.root.as_deref(), common.knobs, Kind::Cli, None)?;
        let lease = open(&mut opener).await.map_err(|e| match e {
            HoldError::ClosedByOwner => {
                "the books were closed just now (Close the books): nothing was done".to_string()
            }
            HoldError::Failed(why) => why,
        })?;
        let run_dir = opener.run_dir().to_path_buf();
        // The books are open: asked about a format upgrade first
        // (INVARIANT 6).
        if common.ask_upgrades && !ASKED_UPGRADES.swap(true, std::sync::atomic::Ordering::SeqCst) {
            let run = run_dir.clone();
            let _ = tokio::task::spawn_blocking(move || offer_upgrades(&run)).await;
        }
        let socket = run_dir.join(ledger_mcp::local::ADMIN_SOCKET);
        // A list is quick; a create or an adoption takes what it takes.
        let wait = matches!(request, Request::List | Request::TokenList).then_some(LIST_WAIT);
        let answer =
            tokio::task::spawn_blocking(move || admin::call_within(&socket, request, wait))
                .await
                .map_err(|e| e.to_string())?
                .map_err(|e| e.to_string());
        drop(lease);
        answer.map(|a| (a, run_dir))
    });
    // Past its deadline, the command ends: nothing pending holds it.
    rt.shutdown_timeout(Duration::from_secs(1));
    outcome
}

/// Where the books stand while `close` waits for them to close.
#[derive(Debug, PartialEq, Eq)]
enum Closing {
    /// No host, and no server finishing: closed.
    Closed,
    /// A new host, opened by something since: closed, then opened again.
    Reopened,
    /// Still on their way (a server finishing, or a host not answering).
    Going,
}

/// What the host's answer (`asked`) and the server's lock say.
fn closing(
    asked: &Result<Option<HostMsg>, String>,
    server_lock_held: impl FnOnce() -> bool,
) -> Closing {
    match asked {
        Ok(None) if server_lock_held() => Closing::Going,
        Ok(None) => Closing::Closed,
        Ok(Some(HostMsg::Leases { .. })) => Closing::Reopened,
        _ => Closing::Going,
    }
}

/// Whether `deadline` has passed at `now`.
fn past(deadline: std::time::Instant, now: std::time::Instant) -> bool {
    now > deadline
}

/// What `doctor` says of books no host holds.
fn no_host(server_lock_held: bool) -> &'static str {
    if server_lock_held {
        "The books are closing: a books server is finishing its last call."
    } else {
        "The books are closed."
    }
}

/// `wanderbooks serve`: hold the books open until Ctrl-C.
pub fn serve(
    prog: &str,
    args: impl IntoIterator<Item = String>,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut common = Common::default();
    let mut server_flags: Vec<String> = Vec::new();
    let mut args = args.into_iter().peekable();
    while let Some(arg) = args.next() {
        if common.take(&arg, &mut args)? {
            continue;
        }
        match arg.as_str() {
            // What this server is told (P9a): passed through the host.
            "--isolation" => {
                let level = args
                    .next()
                    .ok_or("--isolation needs same_user or enforced")?;
                if !matches!(
                    level.as_str(),
                    "same_user" | "advisory" | "service_user" | "enforced"
                ) {
                    eprintln!("error: --isolation takes same_user or enforced, not {level:?}");
                    return Err(exit(2));
                }
                server_flags.push(arg);
                server_flags.push(level);
            }
            "--http" => {
                let addr = match args.peek() {
                    Some(v) if !v.starts_with('-') => args.next().unwrap_or_default(),
                    _ => "127.0.0.1:7411".to_string(),
                };
                // Before a host starts: the door is loopback only (3.9, P9a).
                let addr = match ledger_mcp::local::tunnel_address(&addr) {
                    Ok(addr) => addr,
                    Err(why) => {
                        eprintln!("error: {why}");
                        return Err(exit(2));
                    }
                };
                server_flags.push(arg);
                server_flags.push(addr.to_string());
            }
            "--help" | "-h" => {
                eprintln!("usage: {prog} [--root DIR] [--isolation LEVEL] [--http [ADDR]]");
                eprintln!("  Opens the books and holds them open until Ctrl-C. They close 10 s");
                eprintln!("  after nothing else (the app, an agent session) holds them.");
                eprintln!("  --isolation same_user | enforced: how books.start says these books");
                eprintln!("    are kept from agents (enforced: the owner's own machine, P9a).");
                eprintln!("  --http [ADDR]: also serve the MCP door on loopback only (default");
                eprintln!("    127.0.0.1:7411), for a TLS proxy or a tunnel in front of it; any");
                eprintln!("    other address is refused.");
                return Ok(());
            }
            other => {
                eprintln!("unknown argument: {other}");
                eprintln!("usage: {prog}");
                return Err(exit(2));
            }
        }
    }
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    let outcome = rt.block_on(async {
        let mut opener = Opener::new(
            common.root.as_deref(),
            common.knobs,
            Kind::Serve,
            None,
        )?
        .starting_with(server_flags);
        // Ctrl-C, SIGTERM and SIGHUP are heard from the start, whatever
        // serve is doing (opening, waiting to try again): none is missed.
        let stop = std::sync::Arc::new(tokio::sync::Notify::new());
        for kind in [
            tokio::signal::unix::SignalKind::interrupt(),
            tokio::signal::unix::SignalKind::terminate(),
            tokio::signal::unix::SignalKind::hangup(),
        ] {
            let mut signals = tokio::signal::unix::signal(kind)?;
            let stop = stop.clone();
            tokio::spawn(async move {
                while signals.recv().await.is_some() {
                    stop.notify_one();
                }
            });
        }
        let let_go = || {
            println!("Let go of the books.");
            Ok(())
        };
        let mut said_open = false;
        loop {
            println!("Opening the books…");
            let opened = tokio::select! {
                opened = open(&mut opener) => opened,
                () = stop.notified() => return let_go(),
            };
            let mut lease = match opened {
                Ok(lease) => lease,
                Err(HoldError::ClosedByOwner) => {
                    println!("The books were closed (Close the books).");
                    return Ok(());
                }
                Err(HoldError::Failed(why)) => {
                    eprintln!("error: {why}");
                    return Err(exit(1));
                }
            };
            if !said_open {
                println!(
                    "The books are open. Press Ctrl-C to let go of them: they close when nothing else holds them."
                );
                said_open = true;
                // Asked about a format upgrade as they open (INVARIANT 6).
                // A prompt is answered before Ctrl-C lets go: never left
                // with the terminal's echo off.
                if !ASKED_UPGRADES.swap(true, std::sync::atomic::Ordering::SeqCst) {
                    let run = opener.run_dir().to_path_buf();
                    let _ = tokio::task::spawn_blocking(move || offer_upgrades(&run)).await;
                }
            } else {
                println!("The books are open again.");
            }
            let ended = tokio::select! {
                () = stop.notified() => None,
                msg = lease.next() => Some(msg),
            };
            match ended {
                None => {
                    drop(lease);
                    return let_go();
                }
                Some(Some(HostMsg::Ending {
                    reason: Reason::ClosedByOwner,
                    ..
                })) => {
                    println!("The books were closed (Close the books).");
                    return Ok(());
                }
                Some(Some(HostMsg::Ending {
                    reason: Reason::Restarting,
                    ..
                })) => {
                    println!("The books are restarting (a new version)…");
                }
                Some(Some(HostMsg::Ending {
                    reason: Reason::Closing,
                    ..
                })) => {}
                Some(_) => {
                    println!("The books server stopped; opening the books again…");
                    let waited = tokio::select! {
                        waited = opener.failed() => waited,
                        () = stop.notified() => return let_go(),
                    };
                    if let Err(why) = waited {
                        eprintln!("error: {why}");
                        return Err(exit(1));
                    }
                }
            }
        }
    });
    // A holder that gave up isn't held back by work still pending.
    rt.shutdown_timeout(Duration::from_secs(1));
    outcome
}

/// `wanderbooks close`: close the books now; every holder lets go.
pub fn close(
    prog: &str,
    args: impl IntoIterator<Item = String>,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut common = Common::default();
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        if common.take(&arg, &mut args)? {
            continue;
        }
        match arg.as_str() {
            "--help" | "-h" => {
                eprintln!("usage: {prog}");
                eprintln!("  Closes the books now: the app, agent sessions and `serve` let go.");
                return Ok(());
            }
            other => {
                eprintln!("unknown argument: {other}");
                eprintln!("usage: {prog}");
                return Err(exit(2));
            }
        }
    }
    let root = ledger_store::root::Root::find(common.root.as_deref()).map_err(|e| e.to_string())?;
    let run = root.run_dir().map_err(|e| e.to_string())?;
    let asked = crate::opener::ask_once_within(&run, Ask::Intent(Intent::Close), HOST_WAIT)
        .map_err(|why| {
            // Asked, and no answer: the host may yet close them.
            eprintln!(
                "error: {why}: the books may still close; `wanderbooks doctor` shows whether they did"
            );
            exit(1)
        })?;
    match asked {
        None => {
            println!("The books are already closed.");
            Ok(())
        }
        Some(HostMsg::Done { .. }) => {
            println!("Closing the books: everything holding them lets go…");
            let deadline = std::time::Instant::now() + CLOSE_WAIT;
            loop {
                let asked = crate::opener::ask_once_within(&run, Ask::Leases, HOST_WAIT);
                match closing(&asked, || crate::opener::server_lock_held(&run)) {
                    Closing::Closed => break,
                    Closing::Reopened => {
                        println!("The books were closed; something has opened them again.");
                        return Ok(());
                    }
                    Closing::Going => {}
                }
                if past(deadline, std::time::Instant::now()) {
                    println!("They are still closing (a request is finishing).");
                    return Ok(());
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            println!("The books are closed.");
            Ok(())
        }
        Some(HostMsg::Refused(why)) => {
            eprintln!("error: {why}");
            Err(exit(1))
        }
        Some(other) => {
            eprintln!("error: the books' host answered oddly: {other:?}");
            Err(exit(1))
        }
    }
}

fn books_usage(prog: &str) {
    eprintln!("usage: {prog} list");
    eprintln!("       {prog} create ID [--currency CODE]");
    eprintln!(
        "       {prog} adopt DATA-DIR --id ID [--toml FILE] [--config-json FILE] [--vault DIR --vault-key FILE | --no-vault] [--skip-unknown] [--yes]"
    );
    eprintln!("       {prog} restore BACKUP [--id ID] [--replace] [--yes]");
    eprintln!("       {prog} back-up ID [--new-disk]");
    eprintln!("       {prog} upgrade ID");
    eprintln!("       {prog} extract BOOK ATTACHMENT");
    eprintln!(
        "       {prog} put-aside ID DATA-DIR [--vault DIR --vault-key FILE] [PATH...] [--yes]"
    );
    eprintln!("  list    the books, and whether each is open");
    eprintln!("  create  start new, empty books (base currency USD unless given)");
    eprintln!("  adopt   copy existing books in, verified; the original is left as it was");
    eprintln!(
        "  back-up back the books up now to every backup their settings name (--new-disk: the backup folder's disk is new)"
    );
    eprintln!(
        "  restore bring back a backup (a git repository's address, or BOOK.git in the backup folder), proven whole first: as new books, or replacing books (kept in legacy/)"
    );
    eprintln!(
        "  upgrade give the go-ahead, with the owner passphrase, for books an older wanderbooks wrote to move to this version's journal format (a verified backup with the vault and keys first)"
    );
    eprintln!(
        "  extract read one attachment's text (a PDF, XLSX or text file) and journal it, one at a time; books an older version filed are read this way after an upgrade"
    );
    eprintln!(
        "  put-aside move the old copies of adopted books (their data folder, their vault and its key, and other paths given: settings file, git history) whole into the legacy folder, once the books hold everything they do, byte for byte; nothing is deleted"
    );
}

/// `wanderbooks books list|create|adopt|restore`.
pub fn books(
    prog: &str,
    args: impl IntoIterator<Item = String>,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut common = Common::default();
    let mut words: Vec<String> = Vec::new();
    let mut currency = "USD".to_string();
    let mut id: Option<String> = None;
    let mut toml: Option<PathBuf> = None;
    let mut config_json: Option<PathBuf> = None;
    let mut vault: Option<PathBuf> = None;
    let mut vault_key: Option<PathBuf> = None;
    let mut no_vault = false;
    let mut skip_unknown = false;
    let mut yes = false;
    let mut replace = false;
    let mut new_disk = false;
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        if common.take(&arg, &mut args)? {
            continue;
        }
        let mut value = |what: &str| args.next().ok_or_else(|| format!("{arg} needs {what}"));
        match arg.as_str() {
            "--currency" => currency = value("a currency code")?,
            "--id" => id = Some(value("an id")?),
            "--toml" => toml = Some(PathBuf::from(value("a file")?)),
            "--config-json" => config_json = Some(PathBuf::from(value("a file")?)),
            "--vault" => vault = Some(PathBuf::from(value("a folder")?)),
            "--vault-key" => vault_key = Some(PathBuf::from(value("a file")?)),
            "--no-vault" => no_vault = true,
            "--skip-unknown" => skip_unknown = true,
            "--replace" => replace = true,
            "--new-disk" => new_disk = true,
            "--yes" => yes = true,
            "--help" | "-h" => {
                books_usage(prog);
                return Ok(());
            }
            s if s.starts_with('-') => {
                eprintln!("unknown flag: {s}");
                books_usage(prog);
                return Err(exit(2));
            }
            _ => words.push(arg),
        }
    }
    let words: Vec<&str> = words.iter().map(String::as_str).collect();
    // The owner's command: asked about a format upgrade as the books open
    // (INVARIANT 6), unless it is the upgrade itself.
    common.ask_upgrades = !matches!(words.as_slice(), ["upgrade", ..]);
    let answer = match words.as_slice() {
        ["list"] => with_server(&common, Request::List),
        ["create", new] => with_server(
            &common,
            Request::Create {
                id: (*new).to_string(),
                currency: currency.clone(),
            },
        ),
        ["adopt", source] => {
            let Some(id) = id else {
                eprintln!("error: name the adopted books with --id ID");
                return Err(exit(2));
            };
            return adopt(
                &common,
                Path::new(source),
                id,
                AdoptFiles {
                    toml,
                    config_json,
                    vault,
                    vault_key,
                    no_vault,
                },
                skip_unknown,
                yes,
            );
        }
        ["restore", source] => return restore(&common, source, id, replace, yes),
        ["upgrade", book] => return upgrade(&common, book),
        ["extract", book, attachment] => {
            let attachment_id: u64 = attachment.parse().map_err(|_| {
                eprintln!("error: extract takes an attachment id: {attachment}");
                exit(2)
            })?;
            with_server(
                &common,
                Request::Extract {
                    id: (*book).to_string(),
                    attachment_id,
                },
            )
        }
        ["put-aside", book, source, also @ ..] => {
            let also: Vec<PathBuf> = also.iter().map(PathBuf::from).collect();
            let old = ledger_store::root::OldCopies {
                source: Path::new(source),
                vault_dir: vault.as_deref(),
                vault_key: vault_key.as_deref(),
                also: &also,
                expect_head: None,
            };
            return put_aside(&common, book, old, yes);
        }
        ["back-up", book] => {
            return match back_up_now(&common, book, new_disk) {
                Ok(lines) => {
                    for line in lines {
                        println!("{line}");
                    }
                    Ok(())
                }
                Err(why) => {
                    eprintln!("error: {why}");
                    Err(exit(1))
                }
            };
        }
        _ => {
            books_usage(prog);
            return Err(exit(2));
        }
    };
    match answer {
        Ok(Response::Books(books)) if books.is_empty() => {
            println!("No books yet: `wanderbooks books create ID` starts some.");
            Ok(())
        }
        Ok(Response::Books(books)) => {
            for b in books {
                println!("{}", book_line(&b));
            }
            Ok(())
        }
        Ok(Response::Created { id, problem: None }) => {
            println!("Made new books \"{id}\" ({currency}).");
            Ok(())
        }
        Ok(Response::Created {
            id,
            problem: Some(why),
        }) => {
            eprintln!(
                "error: new books \"{id}\" were made but didn't open: {why}. Closing the books and opening them again tries again."
            );
            Err(exit(1))
        }
        Ok(Response::Extracted {
            attachment_id,
            status,
            pages,
            text_sha256,
        }) => {
            match (&text_sha256, status.as_str()) {
                (Some(sha), "ok") => {
                    println!("Read attachment {attachment_id}: {pages} page(s), text {sha}.")
                }
                (_, "none") => println!(
                    "Read attachment {attachment_id}: no text to read ({pages} page(s)); view its pages as pictures."
                ),
                _ => println!(
                    "Read attachment {attachment_id}: the reader failed ({pages} page(s)); a human must read it."
                ),
            }
            Ok(())
        }
        Ok(Response::Refused(why)) | Err(why) => {
            eprintln!("error: {why}");
            Err(exit(1))
        }
        Ok(other) => {
            eprintln!("error: the books server answered oddly: {other:?}");
            Err(exit(1))
        }
    }
}

/// One book, as `books list` and `doctor` show it.
fn book_line(b: &admin::BookStatus) -> String {
    match (&b.problem, &b.format_upgrade) {
        (Some(why), _) => format!("{}  didn't open: {why}", b.id),
        (None, Some(_)) => format!(
            "{}  open, waiting for a format upgrade (`wanderbooks books upgrade {}`)",
            b.id, b.id
        ),
        (None, None) => format!("{}  open", b.id),
    }
}

/// The projection database a book uses, and the other schemas' files
/// beside it (left as they are for the versions that wrote them, never
/// deleted), as `doctor` says them; None when it didn't open.
fn database_line(b: &admin::BookStatus) -> Option<String> {
    let database = b.database.as_ref()?;
    Some(if b.other_databases.is_empty() {
        format!("database: {database}")
    } else {
        format!(
            "database: {database} (also there, left for the versions that use them: {})",
            b.other_databases.join(", ")
        )
    })
}

/// Back the books `book` up now (the server does it): what came of it, a
/// line each; Err when it couldn't be done, or a target didn't take it.
pub fn back_up_now(common: &Common, book: &str, new_disk: bool) -> Result<Vec<String>, String> {
    match with_server(
        common,
        Request::BackUpNow {
            id: book.to_string(),
            new_disk,
        },
    )? {
        Response::BackedUp { head_seq, targets } => {
            let lines = backed_up_lines(head_seq, &targets);
            if targets.is_empty() || targets.iter().any(|t| t.problem.is_some()) {
                Err(lines.join("\n"))
            } else {
                Ok(lines)
            }
        }
        Response::Refused(why) => Err(why),
        other => Err(format!("the books server answered oddly: {other:?}")),
    }
}

/// What a format upgrade the books `book` wait for means, a line each
/// (plan 3.9, D-39): said before the owner is asked.
pub fn upgrade_explained(book: &str, waiting: &admin::FormatWaiting) -> Vec<String> {
    vec![
        format!(
            "The books \"{book}\" wait for a format upgrade: an older version of wanderbooks wrote them, in journal format {}. The upgrade adds {}.",
            waiting.format,
            waiting.adds.join(", ")
        ),
        "Until you agree they work on their old format: everything that needs nothing new works, and what needs the new format is refused.".into(),
        "Upgrading first makes a backup of the books, their vault and keys included, and proves it whole; then it moves them to the new format, in one step. After that an older version of wanderbooks can't open them.".into(),
    ]
}

/// What a format upgrade did, a line each.
pub fn upgraded_lines(u: &admin::FormatUpgraded) -> Vec<String> {
    if u.added.is_empty() {
        return vec![format!(
            "The books \"{}\" are already in this version's journal format: nothing was changed.",
            u.id
        )];
    }
    let mut lines = Vec::new();
    if let Some(b) = &u.backup {
        lines.push(format!(
            "Backed up first, and proven whole with the vault and keys: the books as they were, through event {} (backup {}).",
            b.head.0,
            short_hash(&b.commit)
        ));
    }
    lines.push(format!(
        "Upgraded the books \"{}\" to this version's journal format ({}): now at event {} ({}).",
        u.id,
        u.added.join(", "),
        u.head.0,
        short_hash(&u.head.1)
    ));
    lines
}

/// Hand the owner's go-ahead for the books `book`'s format upgrade, with
/// the passphrase they typed, to the books' host (the lease socket's
/// consent_format_upgrade; INVARIANT 6), whose server checks it and does
/// it: what came of it, a line each. The books must be open.
fn consent_now(run: &Path, book: &str, passphrase: &str) -> Result<Vec<String>, String> {
    let asked = crate::opener::ask_once_within(
        run,
        Ask::Intent(Intent::ConsentFormatUpgrade {
            book: book.to_string(),
            passphrase: Some(admin::Passphrase(passphrase.to_string())),
        }),
        UPGRADE_WAIT,
    )
    .map_err(|why| {
        format!(
            "{why}, so it may or may not have been done: `wanderbooks books list` says whether the books still wait for it"
        )
    })?;
    match asked {
        Some(HostMsg::Done { detail }) => Ok(detail
            .unwrap_or_default()
            .lines()
            .map(str::to_string)
            .collect()),
        // Refused: nothing was done (the server says so too).
        Some(HostMsg::Refused(why)) => Err(format!("not upgraded: {why}")),
        Some(HostMsg::Uncertain(why)) => Err(why),
        // Sent, and the host gone without an answer (closed, or killed):
        // its server may have done it all the same.
        None => Err(
            "the books' host gave no answer, so it may or may not have been done: `wanderbooks books list` says whether the books still wait for it"
                .into(),
        ),
        Some(other) => Err(format!("the books' host answered oddly: {other:?}")),
    }
}

/// The owner's go-ahead for the books `book`'s format upgrade, with the
/// owner passphrase they typed (the viewer's dialog, `books upgrade`): the
/// books held open meanwhile. What came of it, a line each; or why not,
/// which says whether it may have been done all the same.
pub fn upgrade_format(
    common: &Common,
    book: &str,
    passphrase: &str,
) -> Result<Vec<String>, String> {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|e| e.to_string())?;
    let (book, passphrase) = (
        book.to_string(),
        zeroize::Zeroizing::new(passphrase.to_string()),
    );
    let outcome = rt.block_on(async {
        // Nothing is sent until the books are open.
        let not_sent = |why: String| format!("not upgraded: {why}");
        let mut opener =
            Opener::new(common.root.as_deref(), common.knobs, Kind::Cli, None).map_err(not_sent)?;
        let lease = open(&mut opener).await.map_err(|e| match e {
            HoldError::ClosedByOwner => {
                "not upgraded: the books were closed just now (Close the books): nothing was done"
                    .to_string()
            }
            HoldError::Failed(why) => not_sent(why),
        })?;
        let run = opener.run_dir().to_path_buf();
        let asked = tokio::task::spawn_blocking(move || consent_now(&run, &book, &passphrase))
            .await
            .map_err(|e| e.to_string())?;
        drop(lease);
        asked
    });
    rt.shutdown_timeout(Duration::from_secs(1));
    outcome
}

/// Ask the owner about every book waiting for a format upgrade (INVARIANT
/// 6), the books open: at a terminal, the go-ahead (then the owner
/// passphrase); elsewhere only a note.
fn offer_upgrades(run: &Path) {
    let socket = run.join(ledger_mcp::local::ADMIN_SOCKET);
    let Ok(Response::Books(books)) = admin::call_within(&socket, Request::List, Some(OFFER_WAIT))
    else {
        return;
    };
    for b in books {
        if let Some(waiting) = &b.format_upgrade {
            offer_upgrade(run, &b.id, waiting);
        }
    }
}

/// [`offer_upgrades`] for the books `book`. The question comes unasked for,
/// into a command run for something else, so it is asked only where it is
/// seen and answered: at a terminal, the command's output going to it, and
/// run in the foreground (a command whose output goes to a file or a pipe,
/// or a job in the background, would wait unseen for an answer, and mix
/// the question into its output).
fn offer_upgrade(run: &Path, book: &str, waiting: &admin::FormatWaiting) {
    // SAFETY: isatty, tcgetpgrp and getpgrp have no preconditions.
    let seen = unsafe { libc::isatty(1) == 1 && libc::tcgetpgrp(0) == libc::getpgrp() };
    if !(seen && can_type_a_passphrase()) {
        eprintln!(
            "note: the books \"{book}\" wait for a format upgrade (to {}): until the owner agrees, with `wanderbooks books upgrade {book}` at a terminal or in the app, they work on their old format.",
            waiting.adds.join(", ")
        );
        return;
    }
    for line in upgrade_explained(book, waiting) {
        println!("{line}");
    }
    let not_now = || {
        println!(
            "Not now: the books keep working as they are, and you'll be asked again the next time they open (or run `wanderbooks books upgrade {book}`)."
        );
    };
    if !matches!(confirmed("Upgrade them now?", false), Ok(true)) {
        not_now();
        return;
    }
    let passphrase = match ask_owner_passphrase() {
        Ok(Some(p)) => p,
        Ok(None) => {
            not_now();
            return;
        }
        Err(why) => {
            eprintln!("error: {why}");
            return;
        }
    };
    match consent_now(run, book, &passphrase) {
        Ok(lines) => {
            for line in lines {
                println!("{line}");
            }
        }
        Err(why) => eprintln!("error: {why}"),
    }
}

/// The owner passphrase, typed at the terminal for a format upgrade; None
/// when nothing was typed (no go-ahead). Never echoed or repeated.
fn ask_owner_passphrase() -> Result<Option<zeroize::Zeroizing<String>>, String> {
    let typed = rpassword::prompt_password(
        "The owner passphrase, to give the go-ahead (Enter alone: not now): ",
    )
    .map(zeroize::Zeroizing::new)
    .map_err(|e| format!("nothing was read ({})", e.kind()))?;
    Ok((!typed.is_empty()).then_some(typed))
}

/// `wanderbooks books upgrade ID` (INVARIANT 6): the owner's go-ahead for
/// the books' format upgrade, at a terminal.
fn upgrade(common: &Common, book: &str) -> Result<(), Box<dyn std::error::Error>> {
    let fail = |why: String| -> Result<(), Box<dyn std::error::Error>> {
        eprintln!("error: {why}");
        Err(exit(1))
    };
    let book = match ledger_store::root::BookId::parse(book) {
        Ok(id) => id.to_string(),
        Err(e) => return fail(e.to_string()),
    };
    let listed = match with_server(common, Request::List) {
        Ok(Response::Books(books)) => books,
        Ok(Response::Refused(why)) | Err(why) => return fail(why),
        Ok(other) => return fail(format!("the books server answered oddly: {other:?}")),
    };
    let Some(status) = listed.into_iter().find(|b| b.id == book) else {
        return fail(format!(
            "there are no books called \"{book}\": `wanderbooks books list` lists them"
        ));
    };
    if let Some(why) = status.problem {
        return fail(format!("the books \"{book}\" didn't open: {why}"));
    }
    let Some(waiting) = status.format_upgrade else {
        println!(
            "The books \"{book}\" are already in this version's journal format: nothing to do."
        );
        return Ok(());
    };
    for line in upgrade_explained(&book, &waiting) {
        println!("{line}");
    }
    if !can_type_a_passphrase() {
        return fail(format!(
            "the go-ahead is the owner's, with the owner passphrase typed at a terminal: run `wanderbooks books upgrade {book}` in one (or agree in the app)"
        ));
    }
    let passphrase = match ask_owner_passphrase() {
        Ok(Some(p)) => p,
        Ok(None) => {
            println!(
                "Not upgraded: the books keep working as they are, and you'll be asked again."
            );
            return Ok(());
        }
        Err(why) => return fail(why),
    };
    match upgrade_format(common, &book, &passphrase) {
        Ok(lines) => {
            for line in lines {
                println!("{line}");
            }
            Ok(())
        }
        Err(why) => fail(why),
    }
}

/// "New books…": new, empty books `id` in `currency` (the server makes
/// them).
pub fn new_books(common: &Common, id: &str, currency: &str) -> Result<String, String> {
    match with_server(
        common,
        Request::Create {
            id: id.trim().to_string(),
            currency: currency.trim().to_string(),
        },
    )? {
        Response::Created { id, problem: None } => {
            Ok(format!("Made new books \"{id}\" ({}).", currency.trim()))
        }
        Response::Created {
            id,
            problem: Some(why),
        } => Err(format!(
            "new books \"{id}\" were made but didn't open: {why}. Closing the books and opening them again tries again."
        )),
        Response::Refused(why) => Err(why),
        other => Err(format!("the books server answered oddly: {other:?}")),
    }
}

/// "Close the books": everything holding them lets go (the lease
/// socket's close intent). What came of it.
pub fn close_books(common: &Common) -> Result<String, String> {
    let found =
        ledger_store::root::Root::find(common.root.as_deref()).map_err(|e| e.to_string())?;
    let run = found.run_dir().map_err(|e| e.to_string())?;
    match crate::opener::ask_once_within(&run, Ask::Intent(Intent::Close), HOST_WAIT)? {
        None => Ok("The books are closed.".into()),
        Some(HostMsg::Done { detail }) => Ok(detail
            .map(|d| {
                let mut c = d.chars();
                c.next()
                    .map(|first| format!("{}{}.", first.to_uppercase(), c.as_str()))
                    .unwrap_or_default()
            })
            .filter(|d| !d.is_empty())
            .unwrap_or_else(|| "The books are closing.".into())),
        Some(HostMsg::Refused(why)) => Err(why),
        Some(HostMsg::Uncertain(why)) => Err(why),
        Some(other) => Err(format!("the books' host answered oddly: {other:?}")),
    }
}

/// The longest an agent's own command may take to add the books.
const CONNECT_WAIT: Duration = Duration::from_secs(60);

/// How long the viewer's hold waits before trying again after books that
/// wouldn't open.
const VIEWER_RETRY: Duration = Duration::from_secs(30);

/// Hold the books `book` open while the viewer runs (a lease of kind
/// app), on a thread of its own: the window never waits for it. It lets
/// go only when the owner closes the books (and when the viewer ends,
/// with the process); books that stop, or won't open, are tried again.
pub fn hold_for_viewer(common: &Common, book: Option<String>) -> std::thread::JoinHandle<()> {
    let common = common.clone();
    std::thread::spawn(move || {
        let Ok(rt) = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        else {
            return;
        };
        rt.block_on(async {
            let Ok(mut opener) = Opener::new(common.root.as_deref(), common.knobs, Kind::App, book)
            else {
                return;
            };
            loop {
                // The same opener each time round (it knows it held the
                // books, so it never reopens books the owner closed); its
                // failures counted afresh.
                opener.forget_failures();
                loop {
                    let mut lease = match open(&mut opener).await {
                        Ok(lease) => lease,
                        Err(HoldError::ClosedByOwner) => return,
                        Err(HoldError::Failed(_)) => break,
                    };
                    match lease.next().await {
                        Some(HostMsg::Ending {
                            reason: Reason::ClosedByOwner,
                            ..
                        }) => return,
                        Some(HostMsg::Ending {
                            reason: Reason::Restarting | Reason::Closing,
                            ..
                        }) => {}
                        _ => {
                            if opener.failed().await.is_err() {
                                break;
                            }
                        }
                    }
                }
                tokio::time::sleep(VIEWER_RETRY).await;
            }
        });
    })
}

/// What a backup made now came to, a line each.
pub fn backed_up_lines(head_seq: u64, targets: &[admin::BackedUpTarget]) -> Vec<String> {
    if targets.is_empty() {
        return vec![
            "No backups are set up: ask your agent to set the private repository and the backup folder.".into(),
        ];
    }
    targets
        .iter()
        .map(|t| match (&t.problem, t.through_seq) {
            (None, Some(seq)) if seq == head_seq => {
                format!("Backed up to {} (through event {seq}).", t.at)
            }
            (None, Some(seq)) => format!("Backed up to {} through event {seq}.", t.at),
            (None, None) => format!("Not yet backed up to {}.", t.at),
            (Some(why), _) => format!("Not backed up to {}: {why}", t.at),
        })
        .collect()
}

/// `wanderbooks connect claude|codex --book ID`: an agent of the owner's
/// reaches the books.
pub fn connect(
    prog: &str,
    args: impl IntoIterator<Item = String>,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut common = Common::default();
    let mut book: Option<String> = None;
    let mut words: Vec<String> = Vec::new();
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        if common.take(&arg, &mut args)? {
            continue;
        }
        match arg.as_str() {
            "--book" => book = Some(args.next().ok_or("--book needs an id")?),
            "--help" | "-h" => {
                eprintln!("usage: {prog} claude|codex --book ID");
                eprintln!(
                    "  Lets Claude Code or Codex reach the books: keeps their token in a file in the data root and adds the books to the agent's own list."
                );
                return Ok(());
            }
            s if s.starts_with('-') => {
                eprintln!("unknown flag: {s}");
                eprintln!("usage: {prog} claude|codex --book ID");
                return Err(exit(2));
            }
            _ => words.push(arg),
        }
    }
    let (Some(agent), Some(book), 1) = (words.first(), book, words.len()) else {
        eprintln!("usage: {prog} claude|codex --book ID");
        return Err(exit(2));
    };
    // The owner's command (INVARIANT 6).
    common.ask_upgrades = true;
    match connect_and_home(&common, agent, &book) {
        Ok(lines) => {
            for line in lines {
                println!("{line}");
            }
            Ok(())
        }
        Err(why) => {
            eprintln!("error: {why}");
            Err(exit(1))
        }
    }
}

/// Let the agent `agent` ("claude" or "codex") reach the books `book`:
/// their token kept in its token file (made if none is), and `wanderbooks
/// mcp --book <book>` added to the agent's own list as
/// `wanderbooks-<book>`. What was done, a line each.
pub fn connect_agent(common: &Common, agent: &str, book: &str) -> Result<Vec<String>, String> {
    let (cli, label) = match agent {
        "claude" => ("claude", "Claude Code"),
        "codex" => ("codex", "Codex"),
        other => return Err(format!("{other:?}: connect claude or codex")),
    };
    let book = ledger_store::root::BookId::parse(book)
        .map_err(|e| e.to_string())?
        .to_string();
    let program = find_agent_program(cli).ok_or_else(|| {
        format!(
            "{label} isn't installed here (no `{cli}` on the PATH, nor where its installer puts it)"
        )
    })?;
    let mut lines = Vec::new();
    // The token: kept in its file, or made now.
    let found =
        ledger_store::root::Root::find(common.root.as_deref()).map_err(|e| e.to_string())?;
    let run = found.run_dir().map_err(|e| e.to_string())?;
    let account = crate::tokens::Account::new(found.path(), &book, crate::tokens::DEFAULT_NAME);
    if crate::tokens::keeper(&run).get(&account)?.is_none() {
        let scopes = ledger_store::auth::Scope::ALL
            .iter()
            .map(|s| s.as_str().to_string())
            .collect();
        make_token(common, &book, crate::tokens::DEFAULT_NAME, scopes, true)?;
        lines.push(format!(
            "Made the agent token of the books {book}, kept in a token file in the data root."
        ));
    }
    // The agent's own list: the books' entry made anew.
    let exe = crate::start::this_executable()?;
    let name = format!("wanderbooks-{book}");
    let mut serve: Vec<std::ffi::OsString> = vec![
        exe.into_os_string(),
        "mcp".into(),
        "--book".into(),
        book.clone().into(),
    ];
    if common.root.is_some() {
        serve.push("--root".into());
        serve.push(found.path().as_os_str().to_os_string());
    }
    serve.extend(common.knobs.args().into_iter().map(Into::into));
    let scope: &[&str] = if agent == "claude" {
        &["--scope", "user"]
    } else {
        &[]
    };
    // Each call within a minute: an agent's command that hangs is said,
    // never waited on for good.
    let run_cli = |args: Vec<std::ffi::OsString>| -> Result<std::process::Output, String> {
        let mut child = std::process::Command::new(&program)
            .args(args)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .map_err(|e| format!("{label} couldn't be run: {}", e.kind()))?;
        let deadline = std::time::Instant::now() + CONNECT_WAIT;
        loop {
            if child.try_wait().map_err(|e| e.to_string())?.is_some() {
                return child.wait_with_output().map_err(|e| e.to_string());
            }
            if std::time::Instant::now() > deadline {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("{label} didn't answer in time: nothing was added"));
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    };
    let mut remove: Vec<std::ffi::OsString> = vec!["mcp".into(), "remove".into()];
    remove.extend(scope.iter().map(Into::into));
    remove.push(name.clone().into());
    let _ = run_cli(remove);
    let mut add: Vec<std::ffi::OsString> = vec!["mcp".into(), "add".into()];
    add.extend(scope.iter().map(Into::into));
    add.push(name.clone().into());
    add.push("--".into());
    add.extend(serve);
    let out = run_cli(add)?;
    if !out.status.success() {
        let said: String = String::from_utf8_lossy(&out.stderr)
            .trim()
            .chars()
            .take(300)
            .collect();
        return Err(format!("{label} didn't add the books: {said}"));
    }
    lines.push(format!(
        "{label} reaches the books {book} as \"{name}\" (restart {label} to see it)."
    ));
    Ok(lines)
}

/// Where an agent's program is: on the PATH, else where its own
/// installer puts it (a viewer started from the desktop has a short
/// PATH). `home` is the account's home (`$HOME`: a test's own home is never
/// the account's).
pub fn find_agent_in(
    cli: &str,
    path: Option<&std::ffi::OsStr>,
    home: Option<&Path>,
) -> Option<PathBuf> {
    let usual = [
        ".local/bin",
        ".npm-global/bin",
        ".bun/bin",
        ".cargo/bin",
        ".volta/bin",
    ]
    .into_iter()
    .filter_map(|d| home.map(|h| h.join(d)))
    .chain(
        ["/usr/local/bin", "/opt/homebrew/bin"]
            .into_iter()
            .map(PathBuf::from),
    );
    path.map(|p| std::env::split_paths(p).collect::<Vec<_>>())
        .unwrap_or_default()
        .into_iter()
        .chain(usual)
        .filter(|d| d.is_absolute())
        .map(|d| d.join(cli))
        .find(|p| is_program(p))
}

/// Whether `p` is a file this account can run.
fn is_program(p: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(p).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
    }
    #[cfg(not(unix))]
    {
        p.is_file()
    }
}

/// This account's home, from the environment.
fn home_dir_env() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .filter(|h| h.is_absolute())
}

/// The agent program `cli` ("claude" or "codex") for this account.
fn find_agent_program(cli: &str) -> Option<PathBuf> {
    find_agent_in(
        cli,
        std::env::var_os("PATH").as_deref(),
        home_dir_env().as_deref(),
    )
}

/// How a person gets an agent's program: the vendor's own installer,
/// never bundled (D-27).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Installer {
    /// The agent's name.
    pub label: &'static str,
    /// The vendor's install command, to run in a terminal.
    pub command: &'static str,
    /// The vendor's page about it.
    pub page: &'static str,
}

/// The vendor's installer for `agent` ("claude" or "codex").
pub fn installer(agent: &str) -> Option<Installer> {
    match agent {
        "claude" => Some(Installer {
            label: "Claude Code",
            command: if cfg!(windows) {
                "irm https://claude.ai/install.ps1 | iex"
            } else {
                "curl -fsSL https://claude.ai/install.sh | bash"
            },
            page: "https://code.claude.com/docs/en/setup",
        }),
        "codex" => Some(Installer {
            label: "Codex",
            command: "npm install -g @openai/codex",
            page: "https://developers.openai.com/codex/cli",
        }),
        _ => None,
    }
}

/// The folder holding the user's documents: `$XDG_DOCUMENTS_DIR` if
/// absolute, else `Documents` in the home (the same word on every system
/// this program runs on).
pub fn documents_dir() -> Option<PathBuf> {
    std::env::var_os("XDG_DOCUMENTS_DIR")
        .map(PathBuf::from)
        .filter(|d| d.is_absolute())
        .or_else(|| home_dir_env().map(|h| h.join("Documents")))
}

/// The books home of `book` (D-27): a visible folder,
/// `<Documents>/Wanderbooks/<book>`, where an agent works on that book. It
/// holds the agent's pointer and settings for the book and nothing of the
/// books themselves (see [`make_books_home_in`]). Named by the book's id,
/// which never changes, not by the company's name, which is set after the
/// books are made and can change.
pub fn books_home(book: &str) -> PathBuf {
    books_home_in(
        &documents_dir().unwrap_or_else(|| PathBuf::from("Documents")),
        book,
    )
}

/// [`books_home`] under the given documents folder.
pub fn books_home_in(documents: &Path, book: &str) -> PathBuf {
    documents.join("Wanderbooks").join(book)
}

/// The program an agent's pointer file names: the AppImage's own file when
/// this runs from one (its mount goes away with the process), else this
/// executable as it is on disk.
fn pointer_program() -> Result<PathBuf, String> {
    if let Some(image) = std::env::var_os("APPIMAGE")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute() && p.is_file())
    {
        return Ok(image);
    }
    crate::start::this_executable()
}

/// Make (or refresh) the books home of `book` under `documents`, for the
/// program `exe` (D-27). It holds exactly four files:
///
/// ```text
/// AGENTS.md, CLAUDE.md   three lines saying whose books these are and
///                        how to reach them (written when absent: a
///                        person's edits stay)
/// .mcp.json              the project-level MCP entry: `<exe> mcp --book <id>`
/// .claude/settings.json  allows `mcp__wanderbooks-<id>__*`, so the daily
///                        flow doesn't prompt
/// ```
///
/// No books data, no key, no token, no path into the data root: the entry
/// finds the root the way every `wanderbooks` command does. The two JSON
/// files are merged into, so anything else a person keeps in them stays.
/// Trust in the folder is the person's to give: nothing here marks it
/// trusted for any agent (Q-7).
pub fn make_books_home_in(
    documents: &Path,
    book: &str,
    company: Option<&str>,
    exe: &Path,
) -> Result<PathBuf, String> {
    let book = ledger_store::root::BookId::parse(book)
        .map_err(|e| e.to_string())?
        .to_string();
    if !documents.is_absolute() {
        return Err("there is no documents folder to make the books home in".into());
    }
    let home = books_home_in(documents, &book);
    let write_err =
        |what: &Path, e: std::io::Error| format!("couldn't write {}: {}", what.display(), e.kind());
    std::fs::create_dir_all(home.join(".claude")).map_err(|e| write_err(&home, e))?;
    let name = format!("wanderbooks-{book}");
    // The three lines, for both agents' file names.
    let whose = company
        .map(str::trim)
        .filter(|c| !c.is_empty())
        .unwrap_or(&book);
    let text = format!(
        "These are {whose}'s books.\nConnect the wanderbooks MCP and call `books.start`.\nThe books live in the app's private folder, not here.\n"
    );
    for file in ["AGENTS.md", "CLAUDE.md"] {
        let path = home.join(file);
        if !path.exists() {
            std::fs::write(&path, &text).map_err(|e| write_err(&path, e))?;
        }
    }
    let merge = |path: PathBuf,
                 edit: &dyn Fn(&mut serde_json::Map<String, serde_json::Value>)|
     -> Result<(), String> {
        let mut doc = std::fs::read_to_string(&path)
            .ok()
            .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
            .filter(serde_json::Value::is_object)
            .unwrap_or_else(|| serde_json::json!({}));
        if let Some(map) = doc.as_object_mut() {
            edit(map);
        }
        let mut out = serde_json::to_string_pretty(&doc).map_err(|e| e.to_string())?;
        out.push('\n');
        std::fs::write(&path, out).map_err(|e| write_err(&path, e))
    };
    let entry = serde_json::json!({
        "command": exe.to_string_lossy(),
        "args": ["mcp", "--book", book],
    });
    merge(home.join(".mcp.json"), &|doc| {
        let servers = doc
            .entry("mcpServers")
            .or_insert_with(|| serde_json::json!({}));
        if let Some(servers) = servers.as_object_mut() {
            servers.insert(name.clone(), entry.clone());
        }
    })?;
    let rule = format!("mcp__{name}__*");
    merge(home.join(".claude").join("settings.json"), &|doc| {
        let perms = doc
            .entry("permissions")
            .or_insert_with(|| serde_json::json!({}));
        if let Some(perms) = perms.as_object_mut() {
            let allow = perms
                .entry("allow")
                .or_insert_with(|| serde_json::json!([]));
            if let Some(allow) = allow.as_array_mut()
                && !allow.iter().any(|r| r.as_str() == Some(rule.as_str()))
            {
                allow.push(serde_json::Value::String(rule.clone()));
            }
        }
    })?;
    Ok(home)
}

/// [`make_books_home_in`] for this account: its documents folder and this
/// program.
pub fn make_books_home(book: &str, company: Option<&str>) -> Result<PathBuf, String> {
    let documents = documents_dir().ok_or("this account has no documents folder")?;
    make_books_home_in(&documents, book, company, &pointer_program()?)
}

/// [`connect_agent`], and the books home made beside it (D-27: the agent's
/// own list gets the user-scope entry as ever; the home adds the
/// project-level one). What was done, a line each.
pub fn connect_and_home(common: &Common, agent: &str, book: &str) -> Result<Vec<String>, String> {
    let mut lines = connect_agent(common, agent, book)?;
    lines.push(home_line(book));
    Ok(lines)
}

/// [`new_books`], and the books home made beside them (D-27): the viewer's
/// "New books…". The home is a convenience: its trouble is said, and the
/// books stay made.
pub fn new_books_and_home(common: &Common, id: &str, currency: &str) -> Result<String, String> {
    let made = new_books(common, id, currency)?;
    Ok(format!("{made} {}", home_line(id.trim())))
}

/// The books home made, said in a line (its trouble, too: the home is a
/// convenience, never a reason to fail what was asked).
fn home_line(book: &str) -> String {
    match make_books_home(book, None) {
        Ok(home) => format!(
            "The books home is {}: start your agent there.",
            home.display()
        ),
        Err(why) => format!("The books home wasn't made: {why}."),
    }
}

/// What starting an agent in a books home needs (the dock runs it on a
/// terminal of its own).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AgentStart {
    /// The program, its arguments, the folder it starts in and the
    /// environment it is given.
    Ready {
        /// The agent's name.
        label: &'static str,
        /// The program.
        program: PathBuf,
        /// Its arguments.
        args: Vec<String>,
        /// The books home.
        dir: PathBuf,
        /// Environment to add.
        env: Vec<(String, String)>,
    },
    /// The agent isn't installed: how to get it.
    Missing(Installer),
}

/// Start `agent` ("claude" or "codex") on `book` (D-27): make sure the
/// books home is there, and say how to run the agent in it. Claude Code
/// and Codex ask about trusting the folder themselves the first time (Q-7:
/// the app marks nothing trusted).
pub fn start_agent(agent: &str, book: &str) -> Result<AgentStart, String> {
    let documents = documents_dir().ok_or("this account has no documents folder")?;
    start_agent_in(
        &documents,
        &pointer_program()?,
        std::env::var_os("PATH").as_deref(),
        home_dir_env().as_deref(),
        agent,
        book,
    )
}

/// [`start_agent`] with its surroundings given: the documents folder, the
/// program the books home points at, the `PATH` and the home that find the
/// agent.
pub fn start_agent_in(
    documents: &Path,
    exe: &Path,
    path: Option<&std::ffi::OsStr>,
    home: Option<&Path>,
    agent: &str,
    book: &str,
) -> Result<AgentStart, String> {
    let how = installer(agent).ok_or_else(|| format!("{agent:?}: claude or codex"))?;
    let cli = if agent == "claude" { "claude" } else { "codex" };
    let dir = make_books_home_in(documents, book, None, exe)?;
    let Some(program) = find_agent_in(cli, path, home) else {
        return Ok(AgentStart::Missing(how));
    };
    Ok(AgentStart::Ready {
        label: how.label,
        program,
        args: Vec::new(),
        dir,
        env: vec![
            ("TERM".into(), "xterm-256color".into()),
            ("COLORTERM".into(), "truecolor".into()),
        ],
    })
}

/// Where a first run stands (P11b: onboarding with no terminal): whether
/// the books' private folder is there, which books it holds, and whether
/// the owner passphrase is set.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FirstRun {
    /// The private folder (data root) exists and can be opened.
    pub root: bool,
    /// The ids of the books it holds.
    pub books: Vec<String>,
    /// The owner passphrase is set.
    pub passphrase_set: bool,
}

/// [`FirstRun`] for the root `common` names (else the one setup recorded).
pub fn first_run_state(common: &Common) -> FirstRun {
    let Ok(found) = ledger_store::root::Root::find(common.root.as_deref()) else {
        return FirstRun::default();
    };
    FirstRun {
        root: true,
        books: found
            .list_books()
            .map(|b| b.iter().map(|id| id.as_str().to_string()).collect())
            .unwrap_or_default(),
        passphrase_set: found
            .auth_dir()
            .ok()
            .and_then(|a| ledger_store::passphrase::is_set(&a).ok())
            .unwrap_or(false),
    }
}

/// `wanderbooks setup` for a viewer with no terminal: make the books'
/// private folder (`common.root`, else this account's default; plan 3.4).
/// What was done, a line each. The folder's place is not said.
pub fn set_up_root(common: &Common) -> Result<Vec<String>, String> {
    let home = ledger_store::root::account_home().map_err(|e| e.to_string())?;
    let xdg = std::env::var_os("XDG_DATA_HOME").map(PathBuf::from);
    let (_, report) =
        ledger_store::root::Root::setup(&home, common.root.as_deref(), xdg.as_deref())
            .map_err(|e| e.to_string())?;
    Ok(vec![
        if report.created {
            "Made the books' private folder. Only your account can open it."
        } else {
            "The books' private folder is there. Only your account can open it."
        }
        .to_string(),
    ])
}

/// Set the owner passphrase for the first time, from a viewer (a person
/// typed it there, twice). Changing one in force is `wanderbooks
/// passphrase`'s: it asks for the one in force.
pub fn set_first_passphrase(common: &Common, new: &str) -> Result<(), String> {
    use ledger_store::passphrase;
    let found =
        ledger_store::root::Root::find(common.root.as_deref()).map_err(|e| e.to_string())?;
    let auth = found.auth_dir().map_err(|e| e.to_string())?;
    if passphrase::is_set(&auth).map_err(|e| e.to_string())? {
        return Err(
            "an owner passphrase is set already: `wanderbooks passphrase` changes it".into(),
        );
    }
    passphrase::check_length(new).map_err(|e| e.to_string())?;
    passphrase::set(&auth, new, None).map_err(|e| e.to_string())
}

/// Make the token `name` for `book` (the server does) and keep it in its
/// token file (INVARIANT 4: kept first, or taken back).
fn make_token(
    common: &Common,
    book: &str,
    name: &str,
    scopes: Vec<String>,
    replace: bool,
) -> Result<admin::TokenInfo, String> {
    let request = Request::TokenCreate {
        book: book.to_string(),
        name: name.to_string(),
        scopes,
        replace,
    };
    let (answer, run_dir) = with_server_in(common, request)?;
    let (info, text) = match answer {
        Response::TokenCreated { token, text } => (token, zeroize::Zeroizing::new(text.0)),
        Response::Refused(why) => return Err(why),
        other => return Err(format!("the books server answered oddly: {other:?}")),
    };
    let root = run_dir.parent().unwrap_or(&run_dir).to_path_buf();
    let account = crate::tokens::Account::new(&root, book, name);
    // Kept, and read back as kept: else taken back (INVARIANT 4).
    let keeper = crate::tokens::keeper(&run_dir);
    let kept = keeper
        .set(&account, &text)
        .map_err(String::from)
        .and_then(|()| match keeper.get(&account) {
            Ok(Some(back)) if back == *text => Ok(()),
            Ok(_) => Err("it didn't give back the token it was given to keep".to_string()),
            Err(e) => Err(e.to_string()),
        });
    drop(text);
    if let Err(why) = kept {
        let undone = with_server(
            common,
            Request::TokenRevoke {
                book: book.to_string(),
                name: name.to_string(),
                // This one only: another may have replaced it.
                id: Some(info.id.clone()),
            },
        );
        let after = match undone {
            Ok(Response::TokenRevoked(_)) => "so it was revoked again",
            _ => "and revoking it again failed too: `wanderbooks token revoke` ends it",
        };
        let replaced = if replace {
            " The token it replaced had already ended."
        } else {
            ""
        };
        return Err(format!(
            "the token was made, but its token file couldn't keep it ({why}), {after}.{replaced}"
        ));
    }
    Ok(info)
}

/// The id a backup is restored as when none is given: its repository's
/// name (`acme.git` → `acme`).
fn restore_id(source: &str) -> Option<String> {
    let last = source
        .trim_end_matches('/')
        .rsplit(['/', ':'])
        .next()?
        .trim_end_matches(".git");
    ledger_store::root::BookId::parse(last)
        .ok()
        .map(|id| id.to_string())
}

/// Ask at the terminal (or take --yes): whether to go ahead.
fn confirmed(question: &str, yes: bool) -> Result<bool, String> {
    if yes {
        return Ok(true);
    }
    if !at_a_terminal() {
        return Err("confirm at a terminal, or run again with --yes".into());
    }
    print!("{question} [y/N] ");
    use std::io::Write;
    let _ = std::io::stdout().flush();
    let mut answer = String::new();
    std::io::stdin()
        .read_line(&mut answer)
        .map_err(|e| e.to_string())?;
    Ok(matches!(answer.trim(), "y" | "Y" | "yes" | "Yes"))
}

/// `wanderbooks books restore` (INVARIANT 3).
fn restore(
    common: &Common,
    source: &str,
    id: Option<String>,
    replace: bool,
    yes: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let fail = |why: String| -> Result<(), Box<dyn std::error::Error>> {
        eprintln!("error: {why}");
        Err(exit(1))
    };
    let plan = match plan_restore(common, source, id, replace, true) {
        Ok(plan) => plan,
        Err(why) => return fail(why),
    };
    for line in &plan.lines {
        println!("{line}");
    }
    match confirmed("Restore it?", yes) {
        Ok(true) => {}
        Ok(false) => {
            println!("Not restored.");
            return Ok(());
        }
        Err(why) => return fail(format!("not restored: {why}")),
    }
    match carry_out_restore(common, &plan) {
        Ok(lines) => {
            for line in lines {
                println!("{line}");
            }
            Ok(())
        }
        Err(why) => fail(why),
    }
}

/// A restore previewed: the backup proven, the id it comes back as, and
/// what it would do, a line each (INVARIANT 3: shown before it is done).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RestorePlan {
    /// The backup, as the server is told it (a folder by its full path).
    pub source: String,
    /// The books' id it comes back as.
    pub id: String,
    /// Whether it replaces the books of that id (kept in legacy/).
    pub replace: bool,
    /// The backup's head, proven: the restore is of this backup only.
    pub head: (u64, String),
    /// The id of the books it holds, if it says.
    pub origin: Option<String>,
    /// What it would do, a line each.
    pub lines: Vec<String>,
}

/// Preview restoring the backup at `source` (`flags`: name the command
/// line's flags in the advice): the id it comes back as is `id`, else the
/// id of the books the backup holds, else the repository's name; refused
/// over books that exist unless `replace`, and replacing none.
pub fn plan_restore(
    common: &Common,
    source: &str,
    id: Option<String>,
    replace: bool,
    flags: bool,
) -> Result<RestorePlan, String> {
    // A folder is named to the server by its full path (the server's
    // working folder isn't this one); anything else is an address.
    let source = match Path::new(source) {
        p if p.exists() => std::path::absolute(p)
            .unwrap_or_else(|_| p.to_path_buf())
            .display()
            .to_string(),
        _ => source.trim().to_string(),
    };
    // One preview (under the id given, or any: it only reports), then the
    // id.
    let probe = id
        .clone()
        .or_else(|| restore_id(&source))
        .unwrap_or_else(|| "restored".to_string());
    let preview = match with_server(
        common,
        Request::Restore {
            id: probe,
            source: source.clone(),
            replace,
            confirmed: false,
            expect_head: None,
        },
    )? {
        Response::RestorePreview(p) => p,
        Response::Refused(why) => return Err(format!("{source}: {why}")),
        other => return Err(format!("the books server answered oddly: {other:?}")),
    };
    let flag = |text: &str| {
        if flags {
            format!(" ({text})")
        } else {
            String::new()
        }
    };
    let Some(id) = id
        .or_else(|| {
            preview
                .origin
                .clone()
                .filter(|o| ledger_store::root::BookId::parse(o).is_ok())
        })
        .or_else(|| restore_id(&source))
    else {
        return Err(format!("name the restored books{}", flag("--id ID")));
    };
    let exists = match with_server(common, Request::List)? {
        Response::Books(books) => books.iter().any(|b| b.id == id),
        Response::Refused(why) => return Err(why),
        other => return Err(format!("the books server answered oddly: {other:?}")),
    };
    if exists && !replace {
        return Err(format!(
            "there are already books called \"{id}\": restore them under another id{}, or replace them{}",
            flag("--id"),
            flag("--replace; they are kept in legacy/")
        ));
    }
    if !exists && replace {
        return Err(format!(
            "there are no books called \"{id}\" to replace: nothing was restored"
        ));
    }
    let mut lines = vec![format!(
        "The backup at {source} is whole: through event {} ({}), {} stored files re-hashed, {} TIN vault records.",
        preview.head.0,
        short_hash(&preview.head.1),
        preview.blobs,
        preview.vault_files
    )];
    if let Some(origin) = &preview.origin {
        lines.push(format!("It is a backup of the books \"{origin}\"."));
    }
    lines.push(if replace {
        format!(
            "It replaces the books \"{id}\": they are closed and kept whole in the legacy folder, not deleted."
        )
    } else {
        format!("It comes back as new books \"{id}\", beside any others.")
    });
    match preview.origin.as_deref() {
        Some(o) if o == id => {}
        Some(o) => lines.push(format!(
            "Under a name other than its books' (\"{o}\"), they won't back up to the private repository those books back up to (that would stop theirs): if these are those books, restore them as \"{o}\"{}; else name a repository of their own.",
            flag(&format!("--id {o}"))
        )),
        None => lines.push(
            "It doesn't say whose books it holds: they won't back up to the private repository their settings name until one of their own is named."
                .into(),
        ),
    }
    Ok(RestorePlan {
        source,
        id,
        replace,
        head: preview.head,
        origin: preview.origin,
        lines,
    })
}

/// Restore as `plan` previewed it (only that backup, at that head): what
/// was done, a line each.
pub fn carry_out_restore(common: &Common, plan: &RestorePlan) -> Result<Vec<String>, String> {
    let request = Request::Restore {
        id: plan.id.clone(),
        source: plan.source.clone(),
        replace: plan.replace,
        confirmed: true,
        expect_head: Some(plan.head.clone()),
    };
    match with_server(common, request)? {
        Response::Restored(r) => {
            let mut lines = vec![format!(
                "Restored as \"{}\": through event {} ({}), {} stored files checked, {} TIN vault records.",
                r.id,
                r.head.0,
                short_hash(&r.head.1),
                r.blobs_verified,
                r.vault_files
            )];
            if let Some(kept) = &r.replaced {
                lines.push(format!(
                    "The books it replaced are kept in the legacy folder as {kept}."
                ));
            }
            if let Some(why) = r.problem {
                return Err(format!(
                    "{}\nThe restored books didn't open: {why}. Closing the books and opening them again tries again.",
                    lines.join("\n")
                ));
            }
            Ok(lines)
        }
        Response::Refused(why) => Err(format!("{}: {why}", plan.source)),
        other => Err(format!("the books server answered oddly: {other:?}")),
    }
}

/// The files an adoption brings in with the books.
struct AdoptFiles {
    toml: Option<PathBuf>,
    config_json: Option<PathBuf>,
    vault: Option<PathBuf>,
    vault_key: Option<PathBuf>,
    no_vault: bool,
}

/// A hash's first twelve characters, for people.
fn short_hash(hash: &str) -> String {
    hash.chars().take(12).collect()
}

/// Where a P1 books dir keeps its vault: `vault/` beside the data dir.
fn vault_beside(source: &Path) -> Option<PathBuf> {
    let source = std::fs::canonicalize(source).ok()?;
    let vault = source.parent()?.join("vault");
    vault.is_dir().then_some(vault)
}

/// Whether standard input is a terminal.
fn at_a_terminal() -> bool {
    // SAFETY: isatty has no preconditions.
    unsafe { libc::isatty(0) == 1 }
}

/// `wanderbooks books adopt` (INVARIANT 3).
fn adopt(
    common: &Common,
    source: &Path,
    id: String,
    files: AdoptFiles,
    skip_unknown: bool,
    yes: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let fail = |why: String| -> Result<(), Box<dyn std::error::Error>> {
        eprintln!("error: {why}");
        Err(exit(1))
    };
    let shown = source.display().to_string();
    if files.no_vault && (files.vault.is_some() || files.vault_key.is_some()) {
        return fail("--no-vault and --vault can't both be given".into());
    }
    match (&files.vault, &files.vault_key) {
        (Some(_), None) => {
            return fail("give the vault's key too, with --vault-key FILE (often ~/.config/wanderbooks/vault.key)".into());
        }
        (None, Some(_)) => return fail("give the vault's folder too, with --vault DIR".into()),
        _ => {}
    }
    if files.vault.is_none()
        && !files.no_vault
        && let Some(beside) = vault_beside(source)
    {
        return fail(format!(
            "there is a TIN vault beside these books ({}): name it with --vault DIR and its key with --vault-key FILE, or give --no-vault to leave it behind",
            beside.display()
        ));
    }
    let read = |path: &Option<PathBuf>| -> Result<Option<String>, String> {
        path.as_ref()
            .map(|p| {
                std::fs::read_to_string(p).map_err(|e| format!("can't read {}: {e}", p.display()))
            })
            .transpose()
    };
    let settings_toml = match read(&files.toml) {
        Ok(t) => t,
        Err(why) => return fail(why),
    };
    let config_json = match read(&files.config_json) {
        Ok(t) => t,
        Err(why) => return fail(why),
    };
    let source = std::path::absolute(source).unwrap_or_else(|_| source.to_path_buf());
    let request = |expect_head: Option<(u64, String)>| Request::Adopt {
        id: id.clone(),
        source: source.clone(),
        settings_toml: settings_toml.clone(),
        config_json: config_json.clone(),
        vault_dir: files.vault.clone(),
        vault_key: files.vault_key.clone(),
        confirmed: expect_head.is_some(),
        expect_head,
        skip_unknown,
    };
    let preview = match with_server(common, request(None)) {
        Ok(Response::Preview(p)) => p,
        Ok(Response::Refused(why)) | Err(why) => return fail(format!("{shown}: {why}")),
        Ok(other) => return fail(format!("the books server answered oddly: {other:?}")),
    };
    println!(
        "Adopting the books at {} as \"{id}\": through event {} ({}).",
        source.display(),
        preview.source_head.0,
        short_hash(&preview.source_head.1)
    );
    if preview.keeps_settings {
        println!("They keep their own settings.");
        if files.toml.is_some() {
            println!("(The settings file given isn't used.)");
        }
    } else if preview.settings.is_empty() {
        println!("No settings move in.");
    } else {
        println!("These settings move in from the settings file:");
        for s in &preview.settings {
            match &s.value {
                Some(v) => println!("  {} = {v}", s.key),
                None => println!("  {} (cleared)", s.key),
            }
        }
    }
    for n in &preview.notes {
        println!("  note: {n}");
    }
    for u in &preview.unknown {
        println!("  not moved in (the books don't know it): {u}");
    }
    if !preview.comments.is_empty() {
        println!(
            "The settings file's comments (not settings: keep what matters as a convention or a decision):"
        );
        for c in &preview.comments {
            println!("  # {c}");
        }
    }
    match &files.vault {
        Some(v) => println!(
            "The TIN vault at {} is copied in with its key ({} records, each checked to open).",
            v.display(),
            preview.vault_records
        ),
        None => println!("No TIN vault is copied."),
    }
    if !preview.unknown.is_empty() && !skip_unknown {
        return fail(
            "the settings file has entries the books don't know (above): take them out, or adopt with --skip-unknown to leave them behind".into(),
        );
    }
    println!("The original is left exactly as it is.");
    if !yes {
        if !at_a_terminal() {
            return fail("not adopted: confirm at a terminal, or run again with --yes".into());
        }
        print!("Adopt them? [y/N] ");
        use std::io::Write;
        std::io::stdout().flush()?;
        let mut answer = String::new();
        std::io::stdin().read_line(&mut answer)?;
        if !matches!(answer.trim(), "y" | "Y" | "yes" | "Yes") {
            println!("Not adopted.");
            return Ok(());
        }
    }
    match with_server(common, request(Some(preview.source_head.clone()))) {
        Ok(Response::Adopted(a)) => {
            for line in adopted_lines(&a) {
                println!("{line}");
            }
            if let Some(why) = a.problem {
                return fail(format!(
                    "the adopted books didn't open: {why}. Closing the books and opening them again tries again."
                ));
            }
            Ok(())
        }
        Ok(Response::Refused(why)) | Err(why) => fail(format!("{shown}: {why}")),
        Ok(other) => fail(format!("the books server answered oddly: {other:?}")),
    }
}

/// `wanderbooks books put-aside` (INVARIANT 8 of the root): what goes
/// aside and what was checked, asked for (or --yes), then moved, only if
/// the old copy is still as the preview read it.
fn put_aside(
    common: &Common,
    book: &str,
    old: ledger_store::root::OldCopies<'_>,
    yes: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let fail = |why: String| -> Result<(), Box<dyn std::error::Error>> {
        eprintln!("error: {why}");
        Err(exit(1))
    };
    if old.vault_dir.is_some() != old.vault_key.is_some() {
        return fail(
            "give the vault's folder and its key together, with --vault DIR --vault-key FILE"
                .into(),
        );
    }
    let root = match ledger_store::root::Root::find(common.root.as_deref()) {
        Ok(root) => root,
        Err(e) => return fail(e.to_string()),
    };
    let id = match ledger_store::root::BookId::parse(book) {
        Ok(id) => id,
        Err(e) => return fail(e.to_string()),
    };
    let preview = match root.put_aside(&id, &old, false) {
        Ok(p) => p,
        Err(e) => return fail(e.to_string()),
    };
    let mut held = format!(
        "its journal through event {} ({}) and its {} stored files",
        preview.head.0,
        short_hash(&preview.head.1),
        preview.blob_files
    );
    if preview.key_checked {
        held.push_str(&format!(
            ", its {} vault records and the vault's key",
            preview.vault_files
        ));
    }
    let alike = if preview.vault_changed.is_empty() {
        "byte for byte"
    } else {
        "byte for byte, or newer"
    };
    println!("The books \"{id}\" hold everything the old copy does, {alike}: {held}.");
    println!(
        "These go aside, each whole in a numbered folder of its own, into the legacy folder as {}:",
        preview.name
    );
    for (k, item) in preview.items.iter().enumerate() {
        println!("  {}  {}", k + 1, item.display());
    }
    if !preview.vault_changed.is_empty() {
        println!(
            "The books have sealed these vault records anew since (theirs is newer; the old ones go aside too):"
        );
        for rel in &preview.vault_changed {
            println!("  {}", rel.display());
        }
    }
    if !preview.left_behind.is_empty() {
        println!(
            "These files beside the stored files were never part of the books (left behind by adoption; they go aside too):"
        );
        for rel in &preview.left_behind {
            println!("  {}", rel.display());
        }
    }
    println!("Nothing is deleted; a list of where each came from is kept with them.");
    if !confirmed("Put them aside?", yes).map_err(|why| {
        eprintln!("error: not put aside: {why}");
        exit(1)
    })? {
        println!("Nothing was put aside.");
        return Ok(());
    }
    let old = ledger_store::root::OldCopies {
        expect_head: Some(&preview.head),
        ..old
    };
    match root.put_aside(&id, &old, true) {
        Ok(done) => {
            println!(
                "Put aside: {} paths, in the legacy folder as {}.",
                done.items.len(),
                done.name
            );
            Ok(())
        }
        Err(e) => fail(e.to_string()),
    }
}

/// What an adoption did, a line each.
fn adopted_lines(a: &admin::Adopted) -> Vec<String> {
    let mut extra = String::new();
    if a.unnamed_blobs > 0 {
        extra.push_str(&format!(
            ", {} the books don't name (kept)",
            a.unnamed_blobs
        ));
    }
    if a.skipped_files > 0 {
        extra.push_str(&format!(
            ", {} unfinished or hidden files left behind",
            a.skipped_files
        ));
    }
    let mut lines = vec![format!(
        "Adopted as \"{}\": {} stored files copied, {} of them checked against the books{extra}.",
        a.id, a.blob_files, a.blobs_verified
    )];
    // A vault is adopted only with its key, so a copied key says a vault
    // came too (with its records, perhaps none).
    if a.key_copied {
        lines.push(format!("TIN vault: {} records and its key.", a.vault_files));
    }
    if a.format_upgraded {
        lines.push(
            "The copy is in this version's journal format (the original keeps its own).".into(),
        );
    }
    if a.settings_imported > 0 {
        lines.push(format!("{} settings moved in.", a.settings_imported));
    }
    lines.push(format!(
        "Head: event {} ({}).",
        a.head.0,
        short_hash(&a.head.1)
    ));
    lines
}

/// How long `doctor` waits for the books server to answer.
const DOCTOR_WAIT: Duration = Duration::from_secs(5);
/// How many of each log's last lines `doctor` shows.
const DOCTOR_LINES: usize = 12;

/// The last `n` lines of a log in `run/`, with the root's path taken out.
fn log_tail(run: &Path, name: &str, root: &Path, n: usize) -> Vec<String> {
    let Ok(text) = std::fs::read_to_string(run.join(name)) else {
        return Vec::new();
    };
    // The root as found, and as it really is (through any link).
    let real = std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    let spellings = [root.display().to_string(), real.display().to_string()];
    let lines: Vec<&str> = text.lines().collect();
    lines[lines.len().saturating_sub(n)..]
        .iter()
        .map(|l| {
            let l = spellings.iter().fold(l.to_string(), |l, s| {
                l.replace(s.as_str(), ledger_store::root::THE_ROOT)
            });
            // A log line can carry a panic's text: no full TIN is shown.
            ledger_core::parties::redact_full_tins(&l).into_owned()
        })
        .collect()
}

/// `wanderbooks doctor`: whether the books are open, who holds them, and
/// what the host and the server last said. It names no folder.
pub fn doctor(
    prog: &str,
    args: impl IntoIterator<Item = String>,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut common = Common::default();
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        if common.take(&arg, &mut args)? {
            continue;
        }
        match arg.as_str() {
            "--help" | "-h" => {
                eprintln!("usage: {prog}");
                eprintln!(
                    "  Says whether the books are open, what holds them, and what went wrong."
                );
                return Ok(());
            }
            other => {
                eprintln!("unknown argument: {other}");
                eprintln!("usage: {prog}");
                return Err(exit(2));
            }
        }
    }
    let root = match ledger_store::root::Root::find(common.root.as_deref()) {
        Ok(root) => root,
        Err(e) => {
            println!("The books' private folder: {e}");
            return Err(exit(1));
        }
    };
    println!("The books' private folder: there, and only your account can open it.");
    println!("Isolation: same_user (advisory): hidden from agents, not locked against them.");
    match root
        .auth_dir()
        .map_err(|e| e.to_string())
        .and_then(|a| ledger_store::passphrase::is_set(&a).map_err(|e| e.to_string()))
    {
        Ok(true) => println!("The owner passphrase: set."),
        Ok(false) => println!(
            "The owner passphrase: not set yet: set it in a terminal with `wanderbooks passphrase`."
        ),
        Err(why) => println!("The owner passphrase: {why}"),
    }
    let run = root.run_dir().map_err(|e| e.to_string())?;
    match crate::opener::ask_once_within(&run, Ask::Leases, DOCTOR_WAIT) {
        Ok(None) => println!("{}", no_host(crate::opener::server_lock_held(&run))),
        Ok(Some(HostMsg::Leases { ready, leases })) => {
            if ready {
                println!("The books are open.");
                // Each book, and why one didn't open.
                let socket = run.join(ledger_mcp::local::ADMIN_SOCKET);
                // A quick look: a wedged server doesn't hang the doctor.
                let listed = admin::call_within(&socket, Request::List, Some(DOCTOR_WAIT));
                if let Err(why) = &listed {
                    println!("  The books server doesn't answer: {why}");
                }
                if let Ok(admin::Response::Books(books)) = listed {
                    for b in books {
                        println!("  {}", book_line(&b));
                        if let Some(line) = database_line(&b) {
                            println!("    {line}");
                        }
                    }
                    // Agents' tokens: how many each book has.
                    match admin::call_within(&socket, Request::TokenList, Some(DOCTOR_WAIT)) {
                        Ok(admin::Response::Tokens(tokens)) if tokens.is_empty() => println!(
                            "Agents' tokens: none yet (`wanderbooks token create --book ID`)."
                        ),
                        Ok(admin::Response::Tokens(tokens)) => {
                            println!("Agents' tokens:");
                            for t in tokens {
                                println!("  {}  {}  {}", t.book, t.name, t.scopes.join(","));
                            }
                        }
                        Ok(admin::Response::Refused(why)) => println!("Agents' tokens: {why}"),
                        Ok(_) | Err(_) => {}
                    }
                }
            } else {
                println!("The books are opening (the books server isn't ready yet).");
            }
            if leases.is_empty() {
                println!("Nothing holds them: they close soon.");
            } else {
                println!("Held by:");
                for l in leases {
                    match l.book {
                        Some(book) => println!("  {} (pid {}), on {book}", l.kind.label(), l.pid),
                        None => println!("  {} (pid {})", l.kind.label(), l.pid),
                    }
                }
            }
        }
        Ok(Some(HostMsg::Ending { .. })) => println!("The books are closing."),
        Ok(Some(other)) => println!("The books' host answered oddly: {other:?}"),
        Err(why) => println!("The books' host can't be asked: {why}"),
    }
    for (name, what) in [
        (crate::start::HOST_LOG, "The host"),
        (crate::host::SERVER_LOG, "The books server"),
    ] {
        let tail = log_tail(&run, name, root.path(), DOCTOR_LINES);
        if !tail.is_empty() {
            println!("{what}, last said:");
            for line in tail {
                println!("  {line}");
            }
        }
    }
    Ok(())
}

fn token_usage(prog: &str) {
    eprintln!("usage: {prog} create --book ID [--name NAME] [--scopes LIST] [--replace]");
    eprintln!("       {prog} list");
    eprintln!("       {prog} revoke --book ID [--name NAME]");
    eprintln!("       {prog} check --book ID [--name NAME]");
    eprintln!("       {prog} print --book ID [--name NAME]");
    eprintln!("  create  make an agent's token for the books ID and keep it in its");
    eprintln!("          token file, where `wanderbooks mcp --book ID` finds it");
    eprintln!("          (NAME: agent unless given; LIST: read,write,post,admin unless");
    eprintln!("          given; --replace ends a token of the same name)");
    eprintln!("  list    the tokens there are (never their text)");
    eprintln!("  revoke  end a token, at once, and forget it here");
    eprintln!("  check   whether the token kept here is the books' live one (never");
    eprintln!("          its text: exit 0 if it is)");
    eprintln!("  print   the token's text, for a client that sends it itself");
}

/// `wanderbooks token create|list|revoke|check|print`.
pub fn token(
    prog: &str,
    args: impl IntoIterator<Item = String>,
) -> Result<(), Box<dyn std::error::Error>> {
    use ledger_store::auth::Scope;
    let mut common = Common::default();
    let mut words: Vec<String> = Vec::new();
    let mut book: Option<String> = None;
    let mut name = crate::tokens::DEFAULT_NAME.to_string();
    let mut scopes: Vec<String> = Scope::ALL.iter().map(|s| s.as_str().to_string()).collect();
    let mut replace = false;
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        if common.take(&arg, &mut args)? {
            continue;
        }
        let mut value = |what: &str| args.next().ok_or_else(|| format!("{arg} needs {what}"));
        match arg.as_str() {
            "--book" => book = Some(value("an id")?),
            "--name" => name = value("a name")?,
            "--scopes" => {
                let list = value("a list such as read,write")?;
                let mut out = Vec::new();
                for part in list.split(',').map(str::trim).filter(|p| !p.is_empty()) {
                    match Scope::parse(part) {
                        Some(s) => out.push(s.as_str().to_string()),
                        None => {
                            eprintln!(
                                "error: {part:?} isn't a scope: use read, write, post or admin"
                            );
                            return Err(exit(2));
                        }
                    }
                }
                scopes = out;
            }
            "--replace" => replace = true,
            "--help" | "-h" => {
                token_usage(prog);
                return Ok(());
            }
            s if s.starts_with('-') => {
                eprintln!("unknown flag: {s}");
                token_usage(prog);
                return Err(exit(2));
            }
            _ => words.push(arg),
        }
    }
    // The owner's command (INVARIANT 6).
    common.ask_upgrades = true;
    let fail = |why: String| -> Box<dyn std::error::Error> {
        eprintln!("error: {why}");
        exit(1)
    };
    let need_book = || -> Result<String, Box<dyn std::error::Error>> {
        let Some(book) = book.clone() else {
            eprintln!("error: which books? give --book ID");
            return Err(exit(2));
        };
        ledger_store::root::BookId::parse(&book)
            .map(|id| id.to_string())
            .map_err(|e| fail(e.to_string()))
    };
    let words: Vec<&str> = words.iter().map(String::as_str).collect();
    match words.as_slice() {
        ["create"] => {
            let book = need_book()?;
            let info = make_token(&common, &book, &name, scopes, replace).map_err(fail)?;
            println!(
                "Made the token {:?} for the books {} ({}), kept in a token file in the data root.",
                info.name,
                info.book,
                info.scopes.join(", ")
            );
            println!(
                "An agent reaches the books with `wanderbooks mcp --book {}{}`.",
                info.book,
                if info.name == crate::tokens::DEFAULT_NAME {
                    String::new()
                } else {
                    format!(" --token {}", info.name)
                }
            );
            Ok(())
        }
        ["list"] => match with_server(&common, Request::TokenList).map_err(fail)? {
            Response::Tokens(tokens) if tokens.is_empty() => {
                println!("No tokens yet: `wanderbooks token create --book ID` makes one.");
                Ok(())
            }
            Response::Tokens(tokens) => {
                for t in tokens {
                    println!(
                        "{}  {}  {}  made {}",
                        t.book,
                        t.name,
                        t.scopes.join(","),
                        t.created
                    );
                }
                Ok(())
            }
            Response::Refused(why) => Err(fail(why)),
            other => Err(fail(format!("the books server answered oddly: {other:?}"))),
        },
        ["revoke"] => {
            let book = need_book()?;
            let request = Request::TokenRevoke {
                book: book.clone(),
                name: name.clone(),
                id: None,
            };
            let (answer, run_dir) = with_server_in(&common, request).map_err(fail)?;
            match answer {
                Response::TokenRevoked(t) => {
                    let root = run_dir.parent().unwrap_or(&run_dir).to_path_buf();
                    let account = crate::tokens::Account::new(&root, &book, &name);
                    let forgot = crate::tokens::keeper(&run_dir).delete(&account);
                    println!("Revoked the token {:?} for the books {}.", t.name, t.book);
                    if let Err(why) = forgot {
                        eprintln!(
                            "warning: its token file still holds its text ({why}); it opens nothing now"
                        );
                    }
                    Ok(())
                }
                Response::Refused(why) => Err(fail(why)),
                other => Err(fail(format!("the books server answered oddly: {other:?}"))),
            }
        }
        ["check"] => {
            let book = need_book()?;
            let opener = Opener::new(common.root.as_deref(), common.knobs, Kind::Cli, None)
                .map_err(|e| fail(e.to_string()))?;
            let run_dir = opener.run_dir().to_path_buf();
            let root = run_dir.parent().unwrap_or(&run_dir).to_path_buf();
            let account = crate::tokens::Account::new(&root, &book, &name);
            // Its SHA-256 alone leaves here (INVARIANT 4).
            let kept = crate::tokens::keeper(&run_dir)
                .get(&account)
                .map_err(|e| {
                    fail(format!(
                        "the token file couldn't give the token {name:?} of the books {book}: {e}"
                    ))
                })?
                .map(|text| {
                    let text = zeroize::Zeroizing::new(text);
                    ledger_store::journal::sha256_hex(text.as_bytes())
                });
            let request = Request::TokenCheck {
                book: book.clone(),
                name: name.clone(),
                sha256: kept.clone().unwrap_or_default(),
            };
            let (live, matches) = match with_server(&common, request).map_err(fail)? {
                Response::TokenChecked { live, matches } => (live, matches),
                Response::Refused(why) => {
                    return Err(fail(format!(
                        "the books server didn't take the check (one older than this command, perhaps): {why}"
                    )));
                }
                other => return Err(fail(format!("the books server answered oddly: {other:?}"))),
            };
            let named = if name == crate::tokens::DEFAULT_NAME {
                String::new()
            } else {
                format!(" --name {name}")
            };
            let remake = format!("`wanderbooks token create --book {book}{named} --replace`");
            let said = match (kept.is_some(), live, matches) {
                (true, Some(t), true) => {
                    println!(
                        "The token {name:?} of the books {book} kept in its token file is the live one (made {}).",
                        t.created
                    );
                    return Ok(());
                }
                (true, Some(t), false) => format!(
                    "The token file holds a token {name:?} for the books {book} that isn't the live one (made {}): {remake} makes one and keeps it here; then reconnect the agent.",
                    t.created
                ),
                (true, None, _) => format!(
                    "The token file holds a token {name:?} for the books {book} that they no longer have (it was revoked): `wanderbooks token create --book {book}{named}` makes one."
                ),
                (false, Some(t), _) => format!(
                    "The token file keeps no token {name:?} for the books {book}, though they have a live one (made {}): {remake} makes one kept here.",
                    t.created
                ),
                (false, None, _) => format!(
                    "There is no token {name:?} for the books {book}: `wanderbooks token create --book {book}{named}` makes one."
                ),
            };
            println!("{said}");
            Err(exit(1))
        }
        ["print"] => {
            let book = need_book()?;
            let opener = Opener::new(common.root.as_deref(), common.knobs, Kind::Cli, None)
                .map_err(|e| fail(e.to_string()))?;
            let run_dir = opener.run_dir().to_path_buf();
            let root = run_dir.parent().unwrap_or(&run_dir).to_path_buf();
            let account = crate::tokens::Account::new(&root, &book, &name);
            match crate::tokens::keeper(&run_dir).get(&account) {
                Ok(Some(text)) => {
                    println!("{text}");
                    Ok(())
                }
                Ok(None) => Err(fail(format!(
                    "this computer keeps no token {name:?} for the books {book}: `wanderbooks token create --book {book}` makes one"
                ))),
                Err(why) => Err(fail(why.into())),
            }
        }
        _ => {
            token_usage(prog);
            Err(exit(2))
        }
    }
}

/// Whether a person can type a passphrase here: standard input is a
/// terminal, and the terminal itself can be opened (it is read from).
fn can_type_a_passphrase() -> bool {
    at_a_terminal() && std::fs::File::open("/dev/tty").is_ok()
}

/// Ask for the owner passphrase at the terminal and set it in `auth_dir`
/// (ledger_store::passphrase): the one in force first, if there is one,
/// then the new one twice. Nothing typed is echoed or repeated.
pub fn set_passphrase_at_terminal(auth_dir: &Path) -> Result<(), String> {
    use ledger_store::passphrase;
    if !can_type_a_passphrase() {
        return Err(
            "the owner passphrase is typed by a person, at a terminal: run `wanderbooks passphrase` in one"
                .into(),
        );
    }
    // What is typed is wiped from memory when done with.
    let ask = |prompt: &str| {
        rpassword::prompt_password(prompt)
            .map(zeroize::Zeroizing::new)
            .map_err(|e| format!("nothing was read ({})", e.kind()))
    };
    let current = if passphrase::is_set(auth_dir).map_err(|e| e.to_string())? {
        let c = ask("The owner passphrase in force: ")?;
        if !passphrase::verify(auth_dir, &c).map_err(|e| e.to_string())? {
            return Err(passphrase::PassError::Wrong.to_string());
        }
        Some(c)
    } else {
        None
    };
    let new = ask(&format!(
        "A new owner passphrase ({} characters or more): ",
        passphrase::MIN_CHARS
    ))?;
    passphrase::check_length(&new).map_err(|e| e.to_string())?;
    if *ask("The same again: ")? != *new {
        return Err("the two didn't match: nothing was changed".into());
    }
    passphrase::set(auth_dir, &new, current.as_ref().map(|c| c.as_str())).map_err(|e| e.to_string())
}

/// `wanderbooks passphrase`: set or change the owner passphrase.
pub fn passphrase(
    prog: &str,
    args: impl IntoIterator<Item = String>,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut common = Common::default();
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        if common.take(&arg, &mut args)? {
            continue;
        }
        match arg.as_str() {
            "--help" | "-h" => {
                eprintln!("usage: {prog}");
                eprintln!("  Set the owner passphrase, or change it (the one in force is asked");
                eprintln!("  first). It is what the books ask a person for before anything only");
                eprintln!("  the owner may do. Typed at a terminal, never shown.");
                return Ok(());
            }
            other => {
                eprintln!("unknown argument: {other}");
                return Err(exit(2));
            }
        }
    }
    let fail = |why: String| -> Box<dyn std::error::Error> {
        eprintln!("error: {why}");
        exit(1)
    };
    let root =
        ledger_store::root::Root::find(common.root.as_deref()).map_err(|e| fail(e.to_string()))?;
    let auth = root.auth_dir().map_err(|e| fail(e.to_string()))?;
    set_passphrase_at_terminal(&auth).map_err(fail)?;
    println!("The owner passphrase is set.");
    Ok(())
}

/// A request to a book over the server's socket, with the agent's token
/// from the token file, the books held open meanwhile: its status, the
/// headers wanted, and its body.
fn on_the_socket(
    common: &Common,
    book: &str,
    token_name: &str,
    request: impl FnOnce(&reqwest::Client, String) -> reqwest::RequestBuilder,
) -> Result<(u16, reqwest::header::HeaderMap, Vec<u8>), String> {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|e| e.to_string())?;
    let outcome = rt.block_on(async {
        let mut opener = Opener::new(
            common.root.as_deref(),
            common.knobs,
            Kind::Cli,
            None,
        )?;
        let lease = open(&mut opener).await.map_err(|e| match e {
            HoldError::ClosedByOwner => {
                "the books were closed just now (Close the books): nothing was done".to_string()
            }
            HoldError::Failed(why) => why,
        })?;
        let run_dir = opener.run_dir().to_path_buf();
        let root = run_dir.parent().unwrap_or(&run_dir).to_path_buf();
        let account = crate::tokens::Account::new(&root, book, token_name);
        let keeper = crate::tokens::keeper(&run_dir);
        let token = tokio::task::spawn_blocking(move || keeper.get(&account))
            .await
            .map_err(|e| e.to_string())??
            .ok_or_else(|| {
                format!(
                    "this computer has no token for the books {book} (named {token_name:?}): the human makes one with `wanderbooks token create --book {book}`"
                )
            })?;
        let token = zeroize::Zeroizing::new(token);
        let socket = ledger_store::root::SocketPath::new(&run_dir, ledger_mcp::local::MCP_SOCKET)
            .map_err(|e| format!("can't reach the books server: {e}"))?;
        let http = reqwest::Client::builder()
            .unix_socket(socket.path())
            .no_proxy()
            .build()
            .map_err(|e| e.to_string())?;
        let response = request(&http, format!("http://localhost/b/{book}"))
            .bearer_auth(token.as_str())
            .send()
            .await
            .map_err(|e| format!("the books server didn't answer: {e}"))?;
        let status = response.status().as_u16();
        let headers = response.headers().clone();
        let body = response
            .bytes()
            .await
            .map_err(|e| format!("the books server's answer was cut off: {e}"))?
            .to_vec();
        drop(lease);
        Ok::<_, String>((status, headers, body))
    });
    rt.shutdown_timeout(Duration::from_secs(1));
    outcome
}

/// The flags `upload` and `download` share: the books, the token's name,
/// and the words given.
struct FileArgs {
    common: Common,
    book: String,
    token: String,
    words: Vec<std::ffi::OsString>,
    to: Option<PathBuf>,
}

fn file_args(
    args: impl IntoIterator<Item = std::ffi::OsString>,
    usage: &dyn Fn(),
) -> Result<Option<FileArgs>, Box<dyn std::error::Error>> {
    let mut common = Common::default();
    let mut book = None;
    let mut token = crate::tokens::DEFAULT_NAME.to_string();
    let mut words = Vec::new();
    let mut to = None;
    let mut args = args.into_iter();
    // Flags and their values are text; a file's path, or the folder
    // `--to` names, may be any name the system takes.
    let text = |v: std::ffi::OsString, what: &str| {
        v.into_string()
            .map_err(|_| format!("{what} isn't UTF-8 text"))
    };
    while let Some(arg) = args.next() {
        // After `--`, words only (a file whose name starts with `-`).
        if arg == "--" {
            words.extend(args.by_ref());
            break;
        }
        let Some(flag) = arg.to_str().filter(|a| a.starts_with('-')) else {
            words.push(arg);
            continue;
        };
        let flag = flag.to_string();
        let mut value = |what: &str| args.next().ok_or_else(|| format!("{flag} needs {what}"));
        match flag.as_str() {
            "--root" => common.root = Some(PathBuf::from(value("a folder")?)),
            "--book" => book = Some(text(value("an id")?, "--book's id")?),
            "--token" => token = text(value("a name")?, "--token's name")?,
            "--to" => to = Some(PathBuf::from(value("a folder")?)),
            "--help" | "-h" => {
                usage();
                return Ok(None);
            }
            f if f == FLAG => {
                common.knobs = Knobs::parse(&text(value("a value")?, FLAG)?)?;
            }
            s => {
                eprintln!("unknown flag: {s}");
                usage();
                return Err(exit(2));
            }
        }
    }
    let Some(book) = book else {
        eprintln!("error: which books? give --book ID");
        return Err(exit(2));
    };
    let book = ledger_store::root::BookId::parse(&book)
        .map(|id| id.to_string())
        .map_err(|e| e.to_string())?;
    Ok(Some(FileArgs {
        common,
        book,
        token,
        words,
        to,
    }))
}

/// `wanderbooks upload --book ID UPLOAD FILE`: send a file's bytes for an
/// upload begun with `books.begin_upload` (plan 4.7, D-28: the agent runs
/// it).
pub fn upload(
    prog: &str,
    args: impl IntoIterator<Item = std::ffi::OsString>,
) -> Result<(), Box<dyn std::error::Error>> {
    let usage = || {
        eprintln!("usage: {prog} --book ID [--token NAME] UPLOAD [--] FILE");
        eprintln!("  Send FILE for UPLOAD (from books.begin_upload), with the agent's token.");
    };
    let Some(a) = file_args(args, &usage)? else {
        return Ok(());
    };
    let ([upload, file], None) = (a.words.as_slice(), &a.to) else {
        usage();
        return Err(exit(2));
    };
    let (Some(upload), file) = (
        upload.to_str().filter(|u| ledger_store::uploads::is_id(u)),
        std::path::Path::new(file).display(),
    ) else {
        eprintln!(
            "error: that isn't an upload's id (books.begin_upload gives one: up_ and 32 letters)"
        );
        return Err(exit(2));
    };
    let path = std::path::Path::new(&a.words[1]);
    let fail = |why: String| -> Box<dyn std::error::Error> {
        eprintln!("error: {why}");
        exit(1)
    };
    let max = ledger_store::uploads::MAX_UPLOAD;
    match std::fs::metadata(path) {
        Ok(m) if !m.is_file() => return Err(fail(format!("{file} isn't a file"))),
        Ok(m) if m.len() > max => {
            return Err(fail(format!(
                "{file} is {} bytes; an upload is at most {max}",
                m.len()
            )));
        }
        Ok(_) => {}
        Err(e) => return Err(fail(format!("can't read {file}: {}", e.kind()))),
    }
    let bytes =
        std::fs::read(path).map_err(|e| fail(format!("can't read {file}: {}", e.kind())))?;
    let size = bytes.len();
    let (status, _, body) = on_the_socket(&a.common, &a.book, &a.token, |http, base| {
        http.put(format!("{base}/u/{upload}")).body(bytes)
    })
    .map_err(fail)?;
    let said = String::from_utf8_lossy(&body).to_string();
    if status != 200 {
        return Err(fail(said));
    }
    println!("Sent {size} bytes for {upload}: name it (upload_id) in the tool that files it.");
    Ok(())
}

/// The `Content-Disposition` parameters (RFC 6266): name and value, a
/// quoted value unquoted; a `;` or `=` inside quotes is part of the value.
fn disposition_params(value: &str) -> Vec<(String, String)> {
    let mut params = Vec::new();
    let mut rest = value.split_once(';').map_or("", |(_, r)| r);
    loop {
        rest = rest.trim_start_matches([';', ' ', '\t']);
        let Some((name, after)) = rest.split_once('=') else {
            return params;
        };
        let name = name.trim().to_ascii_lowercase();
        let after = after.trim_start();
        let (v, next) = if let Some(quoted) = after.strip_prefix('"') {
            let mut v = String::new();
            let mut chars = quoted.char_indices();
            let mut end = quoted.len();
            while let Some((i, c)) = chars.next() {
                match c {
                    '\\' => {
                        if let Some((_, e)) = chars.next() {
                            v.push(e);
                        }
                    }
                    '"' => {
                        end = i + 1;
                        break;
                    }
                    c => v.push(c),
                }
            }
            (v, &quoted[end.min(quoted.len())..])
        } else {
            let end = after.find(';').unwrap_or(after.len());
            (after[..end].trim().to_string(), &after[end..])
        };
        params.push((name, v));
        rest = next;
    }
}

/// The longest name a download is saved under, in bytes (under every
/// file system's limit, with room for " (999)").
const SAVED_NAME_MAX: usize = 200;

/// The name a download is saved under: the server's (RFC 6266), its UTF-8
/// `filename*` first; a plain file name, never hidden, nothing invisible
/// in it, at most SAVED_NAME_MAX bytes (its extension kept).
fn saved_name(headers: &reqwest::header::HeaderMap) -> Option<String> {
    let value = headers
        .get(reqwest::header::CONTENT_DISPOSITION)?
        .to_str()
        .ok()?;
    let params = disposition_params(value);
    let utf8 = params
        .iter()
        .find(|(n, _)| n == "filename*")
        .and_then(|(_, v)| v.strip_prefix("UTF-8''"))
        .and_then(percent_decoded);
    let plain = || {
        params
            .iter()
            .find(|(n, _)| n == "filename")
            .map(|(_, v)| v.clone())
    };
    plain_name(&utf8.or_else(plain)?)
}

/// `name` made a plain, visible file name of at most SAVED_NAME_MAX
/// bytes: folders' separators and control characters become `_`,
/// invisible ones go, leading dots and spaces go; None if nothing's left.
fn plain_name(name: &str) -> Option<String> {
    let cleaned: String = name
        .chars()
        .filter(|c| !ledger_store::uploads::is_hidden_char(*c))
        .map(|c| {
            if c == '/' || c == '\\' || c.is_control() {
                '_'
            } else {
                c
            }
        })
        .collect();
    let cleaned = cleaned.trim().trim_start_matches(['.', ' ']).to_string();
    if cleaned.is_empty() {
        return None;
    }
    if cleaned.len() <= SAVED_NAME_MAX {
        return Some(cleaned);
    }
    // Too long: the stem cut on a character boundary, the extension kept
    // (if it is short).
    let (stem, ext) = match cleaned.rsplit_once('.') {
        Some((s, e)) if !s.is_empty() && e.len() <= 16 => (s, format!(".{e}")),
        _ => (cleaned.as_str(), String::new()),
    };
    let mut cut = SAVED_NAME_MAX - ext.len();
    while !stem.is_char_boundary(cut) {
        cut -= 1;
    }
    Some(format!("{}{ext}", stem[..cut].trim_end()))
}

/// `%XX`-escaped UTF-8 (RFC 8187), or None when it isn't.
fn percent_decoded(text: &str) -> Option<String> {
    let mut bytes = Vec::with_capacity(text.len());
    let mut it = text.bytes();
    while let Some(b) = it.next() {
        if b == b'%' {
            let hex = [it.next()?, it.next()?];
            bytes.push(u8::from_str_radix(std::str::from_utf8(&hex).ok()?, 16).ok()?);
        } else {
            bytes.push(b);
        }
    }
    String::from_utf8(bytes).ok()
}

/// Whether `body` is what the server stored: its SHA-256 is the one the
/// server sent (RFC 9530 `Repr-Digest: sha-256=:BASE64:`). No digest, or
/// one that can't be read, is no match.
fn arrived_whole(headers: &reqwest::header::HeaderMap, body: &[u8]) -> bool {
    let sent = headers
        .get(ledger_mcp::local::DIGEST_HEADER)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.trim().strip_prefix("sha-256=:"))
        .and_then(|v| v.strip_suffix(':'))
        .and_then(|v| base64::Engine::decode(&base64::engine::general_purpose::STANDARD, v).ok());
    sent.as_deref() == Some(&<sha2::Sha256 as sha2::Digest>::digest(body)[..])
}

/// `wanderbooks download --book ID ATTACHMENT --to FOLDER`: save a copy of
/// a stored file (one too large for books.get_receipt to return).
pub fn download(
    prog: &str,
    args: impl IntoIterator<Item = std::ffi::OsString>,
) -> Result<(), Box<dyn std::error::Error>> {
    let usage = || {
        eprintln!("usage: {prog} --book ID [--token NAME] ATTACHMENT|EXPORT --to FOLDER");
        eprintln!("  Save a copy of the stored file ATTACHMENT (a number), or of a file the");
        eprintln!("  books made (an EXPORT: ex_ and 32 letters), in FOLDER (a new file).");
    };
    let Some(a) = file_args(args, &usage)? else {
        return Ok(());
    };
    let ([attachment], Some(to)) = (a.words.as_slice(), a.to.clone()) else {
        usage();
        return Err(exit(2));
    };
    // An attachment's id (a number as the books write it), or an
    // export's: each has its own route.
    let route = match attachment.to_str() {
        Some(a) if a.parse::<u64>().is_ok_and(|n| n.to_string() == a) => "a",
        Some(x) if ledger_store::exports::is_id(x) => "x",
        _ => {
            eprintln!(
                "error: that is neither an attachment's id (a number) nor an export's (ex_ and 32 letters)"
            );
            return Err(exit(2));
        }
    };
    let attachment = attachment.to_str().expect("checked above");
    let fail = |why: String| -> Box<dyn std::error::Error> {
        eprintln!("error: {why}");
        exit(1)
    };
    if !to.is_dir() {
        return Err(fail(format!("{} isn't a folder", to.display())));
    }
    let (status, headers, body) = on_the_socket(&a.common, &a.book, &a.token, |http, base| {
        http.get(format!("{base}/{route}/{attachment}"))
    })
    .map_err(fail)?;
    let path = keep(status, &headers, &body, attachment, &to).map_err(fail)?;
    println!("Saved {} ({} bytes).", path.display(), body.len());
    Ok(())
}

/// What `download` does with the server's answer: the server's words if it
/// refused; nothing if the bytes aren't the stored ones; else a new file
/// in `to` (its path).
fn keep(
    status: u16,
    headers: &reqwest::header::HeaderMap,
    body: &[u8],
    attachment: &str,
    to: &std::path::Path,
) -> Result<PathBuf, String> {
    if status != 200 {
        return Err(String::from_utf8_lossy(body).to_string());
    }
    if !arrived_whole(headers, body) {
        return Err(
            "the file arrived damaged (its SHA-256 isn't the stored one): nothing was saved".into(),
        );
    }
    let fallback = match attachment.strip_prefix("ex_") {
        Some(_) => format!("export-{attachment}"),
        None => format!("attachment-{attachment}"),
    };
    let named = saved_name(headers).unwrap_or_else(|| fallback.clone());
    ledger_store::saving::save_new(to, &named, &fallback, body)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn disposition(value: &str) -> reqwest::header::HeaderMap {
        let mut h = reqwest::header::HeaderMap::new();
        h.insert(
            reqwest::header::CONTENT_DISPOSITION,
            reqwest::header::HeaderValue::from_str(value).unwrap(),
        );
        h
    }

    #[test]
    fn a_download_is_saved_under_a_plain_visible_name() {
        let named = |v: &str| saved_name(&disposition(v));
        // The UTF-8 name first, else the plain one.
        assert_eq!(
            named("attachment; filename=\"R_sum_.pdf\"; filename*=UTF-8''R%C3%A9sum%C3%A9.pdf")
                .as_deref(),
            Some("Résumé.pdf")
        );
        assert_eq!(
            named("attachment; filename=\"a b.pdf\"").as_deref(),
            Some("a b.pdf")
        );
        // A parameter inside the quoted name is the name, not a parameter.
        assert_eq!(
            named("attachment; filename=\"x; filename*=UTF-8''.bashrc; y\"").as_deref(),
            Some("x; filename*=UTF-8''.bashrc; y")
        );
        // No folder, nothing hidden, nothing invisible.
        for (given, saved) in [
            (
                "filename*=UTF-8''..%2F..%2F.ssh%2Fauthorized_keys",
                Some("_.._.ssh_authorized_keys"),
            ),
            ("filename*=UTF-8''%2Fetc%2Fpasswd", Some("_etc_passwd")),
            ("filename*=UTF-8''.bashrc", Some("bashrc")),
            ("filename*=UTF-8''rcpt%E2%80%AEfdp.exe", Some("rcptfdp.exe")),
            ("filename*=UTF-8''a%E2%80%8Bb.pdf", Some("ab.pdf")),
            ("filename*=UTF-8''a%0Ab.pdf", Some("a_b.pdf")),
            ("filename*=UTF-8''..", None),
            ("filename*=UTF-8''%20.", None),
            ("filename*=UTF-8''%ZZ", None),
            ("filename=\"\"", None),
        ] {
            assert_eq!(
                named(&format!("attachment; {given}")).as_deref(),
                saved,
                "{given}"
            );
        }
        assert_eq!(saved_name(&reqwest::header::HeaderMap::new()), None);
        // Too long: cut on a character, its extension kept.
        let long = "收".repeat(90) + ".pdf";
        let cut = plain_name(&long).unwrap();
        assert!(
            cut.len() <= SAVED_NAME_MAX && cut.ends_with("收.pdf"),
            "{cut}"
        );
        assert_eq!(
            plain_name(&"a".repeat(SAVED_NAME_MAX)).unwrap().len(),
            SAVED_NAME_MAX
        );
        assert_eq!(percent_decoded("%E2%9C%93").as_deref(), Some("✓"));
        assert_eq!(percent_decoded("%E2%9C"), None, "not UTF-8");
        assert_eq!(percent_decoded("%4"), None);
    }

    #[test]
    fn only_bytes_with_the_digest_sent_arrived_whole() {
        let body = b"%PDF-1.4 stored";
        let digest = base64::Engine::encode(
            &base64::engine::general_purpose::STANDARD,
            <sha2::Sha256 as sha2::Digest>::digest(body),
        );
        let with = |v: &str| {
            let mut h = reqwest::header::HeaderMap::new();
            h.insert(
                ledger_mcp::local::DIGEST_HEADER,
                reqwest::header::HeaderValue::from_str(v).unwrap(),
            );
            h
        };
        assert!(arrived_whole(&with(&format!("sha-256=:{digest}:")), body));
        assert!(!arrived_whole(
            &with(&format!("sha-256=:{digest}:")),
            b"%PDF-1.4 changed"
        ));
        assert!(!arrived_whole(&with(&format!("sha-512=:{digest}:")), body));
        assert!(!arrived_whole(&with(&format!("sha-256={digest}")), body));
        assert!(!arrived_whole(&with("sha-256=:not base64:"), body));
        assert!(!arrived_whole(&reqwest::header::HeaderMap::new(), body));
    }

    #[test]
    fn only_a_whole_file_the_server_gave_is_kept() {
        let dir = tempfile::tempdir().unwrap();
        let body = b"%PDF-1.4 stored";
        let mut h = disposition("attachment; filename=\"r.pdf\"");
        let digest = |b: &[u8]| {
            format!(
                "sha-256=:{}:",
                base64::Engine::encode(
                    &base64::engine::general_purpose::STANDARD,
                    <sha2::Sha256 as sha2::Digest>::digest(b)
                )
            )
        };
        h.insert(
            ledger_mcp::local::DIGEST_HEADER,
            reqwest::header::HeaderValue::from_str(&digest(b"something else")).unwrap(),
        );
        let e = keep(200, &h, body, "7", dir.path()).unwrap_err();
        assert!(e.contains("arrived damaged"), "{e}");
        let e = keep(404, &h, b"there is no such attachment", "7", dir.path()).unwrap_err();
        assert_eq!(e, "there is no such attachment");
        assert_eq!(
            std::fs::read_dir(dir.path()).unwrap().count(),
            0,
            "nothing saved"
        );
        h.insert(
            ledger_mcp::local::DIGEST_HEADER,
            reqwest::header::HeaderValue::from_str(&digest(body)).unwrap(),
        );
        assert_eq!(
            keep(200, &h, body, "7", dir.path()).unwrap(),
            dir.path().join("r.pdf")
        );
        let bare = reqwest::header::HeaderMap::new();
        assert!(
            keep(200, &bare, body, "7", dir.path()).is_err(),
            "no digest, no file"
        );
    }

    #[test]
    fn close_reads_the_books_state_for_what_it_is() {
        let leases = Ok(Some(HostMsg::Leases {
            ready: true,
            leases: vec![],
        }));
        assert_eq!(closing(&Ok(None), || false), Closing::Closed);
        assert_eq!(
            closing(&Ok(None), || true),
            Closing::Going,
            "a server finishing"
        );
        assert_eq!(closing(&leases, || false), Closing::Reopened);
        assert_eq!(closing(&Err("no".into()), || false), Closing::Going);
        assert_eq!(closing(&Ok(Some(HostMsg::Ready)), || false), Closing::Going);
        let now = std::time::Instant::now();
        let later = now + Duration::from_secs(1);
        assert!(!past(later, now));
        assert!(!past(now, now), "not yet at the deadline itself");
        assert!(past(now, later));
    }

    #[test]
    fn doctor_tells_closing_from_closed() {
        assert!(no_host(true).contains("closing"));
        assert_eq!(no_host(false), "The books are closed.");
    }

    fn adopted() -> admin::Adopted {
        admin::Adopted {
            id: "old".into(),
            source_head: (3, "a".repeat(64)),
            head: (4, "0123456789abcdef".repeat(4)),
            blob_files: 5,
            blobs_verified: 4,
            unnamed_blobs: 0,
            skipped_files: 0,
            vault_files: 0,
            key_copied: false,
            settings_imported: 0,
            settings_ignored: false,
            source_lock_created: false,
            format_upgraded: false,
            problem: None,
        }
    }

    #[test]
    fn a_backup_is_restored_under_its_repository_name() {
        for (source, want) in [
            ("/media/disk/backups/acme.git", Some("acme")),
            ("/media/disk/backups/acme.git/", Some("acme")),
            ("git@host:owner/acme-books.git", Some("acme-books")),
            ("ssh://host/owner/acme", Some("acme")),
            ("https://host/owner/acme.git", Some("acme")),
            ("acme.git", Some("acme")),
            ("/media/disk/Backups/Acme.git", None),
            ("/", None),
            ("", None),
        ] {
            assert_eq!(restore_id(source).as_deref(), want, "{source}");
        }
    }

    #[test]
    fn an_adoption_says_what_it_did_and_only_that() {
        let plain = adopted_lines(&adopted());
        assert_eq!(
            plain,
            [
                "Adopted as \"old\": 5 stored files copied, 4 of them checked against the books.",
                "Head: event 4 (0123456789ab).",
            ]
        );
        let all = adopted_lines(&admin::Adopted {
            unnamed_blobs: 1,
            skipped_files: 2,
            vault_files: 3,
            key_copied: true,
            settings_imported: 6,
            format_upgraded: true,
            ..adopted()
        });
        assert_eq!(
            all,
            [
                "Adopted as \"old\": 5 stored files copied, 4 of them checked against the books, 1 the books don't name (kept), 2 unfinished or hidden files left behind.",
                "TIN vault: 3 records and its key.",
                "The copy is in this version's journal format (the original keeps its own).",
                "6 settings moved in.",
                "Head: event 4 (0123456789ab).",
            ]
        );
        // A vault with no records still came, with its key.
        let empty = adopted_lines(&admin::Adopted {
            key_copied: true,
            ..adopted()
        });
        assert!(empty.contains(&"TIN vault: 0 records and its key.".to_string()));
    }

    /// What the owner is told of a format upgrade: what they wait for,
    /// what was done (the backup first, then the format), and that books
    /// already in this version's format had nothing done.
    #[test]
    fn a_format_upgrade_is_told_in_plain_words() {
        let waiting = admin::FormatWaiting {
            format: 1,
            features: Vec::new(),
            adds: vec!["journal format 2".into()],
        };
        let told = upgrade_explained("acme", &waiting).join(" ");
        for says in [
            "\"acme\" wait for a format upgrade",
            "in journal format 1",
            "The upgrade adds journal format 2.",
            "Until you agree they work on their old format",
            "vault and keys included",
            "an older version of wanderbooks can't open them",
        ] {
            assert!(told.contains(says), "{says}: {told}");
        }
        let status =
            |problem: Option<&str>, waiting: Option<admin::FormatWaiting>| admin::BookStatus {
                id: "acme".into(),
                open: problem.is_none(),
                problem: problem.map(str::to_string),
                format_upgrade: waiting,
                database: None,
                other_databases: Vec::new(),
            };
        assert_eq!(book_line(&status(None, None)), "acme  open");
        assert_eq!(
            book_line(&status(None, Some(waiting.clone()))),
            "acme  open, waiting for a format upgrade (`wanderbooks books upgrade acme`)"
        );
        assert_eq!(
            book_line(&status(Some("damaged"), Some(waiting))),
            "acme  didn't open: damaged"
        );
        let done = admin::FormatUpgraded {
            id: "acme".into(),
            added: vec!["journal format 2".into()],
            backup: Some(admin::UpgradeBackup {
                commit: "0123456789abcdef".repeat(2),
                head: (7, "a".repeat(64)),
            }),
            head: (8, "b".repeat(64)),
        };
        assert_eq!(
            upgraded_lines(&done),
            [
                "Backed up first, and proven whole with the vault and keys: the books as they were, through event 7 (backup 0123456789ab).",
                "Upgraded the books \"acme\" to this version's journal format (journal format 2): now at event 8 (bbbbbbbbbbbb).",
            ]
        );
        assert_eq!(
            upgraded_lines(&admin::FormatUpgraded {
                added: Vec::new(),
                backup: None,
                ..done
            }),
            [
                "The books \"acme\" are already in this version's journal format: nothing was changed."
            ]
        );
    }

    /// `doctor` names the database file a book uses, and any other
    /// schema's beside it as left alone; nothing for books that didn't
    /// open.
    #[test]
    fn doctor_names_the_database_in_use_and_the_others_left_alone() {
        let mut b = admin::BookStatus {
            id: "acme".into(),
            open: true,
            problem: None,
            format_upgrade: None,
            database: Some("wanderbooks.v13.db".into()),
            other_databases: Vec::new(),
        };
        assert_eq!(
            database_line(&b).as_deref(),
            Some("database: wanderbooks.v13.db")
        );
        b.other_databases = vec!["wanderbooks.db".into(), "wanderbooks.v14.db".into()];
        assert_eq!(
            database_line(&b).as_deref(),
            Some(
                "database: wanderbooks.v13.db (also there, left for the versions that use them: wanderbooks.db, wanderbooks.v14.db)"
            )
        );
        b.database = None;
        assert_eq!(database_line(&b), None);
    }

    #[test]
    fn the_log_lines_shown_name_no_folder_and_no_tin() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("root");
        std::fs::create_dir(&root).unwrap();
        let run = root.join("run");
        std::fs::create_dir(&run).unwrap();
        std::fs::write(
            run.join("server.log"),
            format!(
                "old line\nthread panicked at {}/books/acme: mid-write 241-40-4915\nlast\n",
                root.display()
            ),
        )
        .unwrap();
        let tail = log_tail(&run, "server.log", &root, 2);
        assert_eq!(tail.len(), 2);
        let text = tail.join("\n");
        assert!(!text.contains(&root.display().to_string()), "{text}");
        assert!(!text.contains("241-40-4915"), "{text}");
        assert!(text.contains(ledger_store::root::THE_ROOT), "{text}");
        assert_eq!(tail[1], "last");
        assert!(log_tail(&run, "none.log", &root, 5).is_empty());
    }
}
