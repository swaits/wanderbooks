//! The owner's commands: `wanderbooks serve`, `close`, `books` and
//! `token` (plan 3.3, 3.9, 5.5).
//!
//! ```text
//! GOAL: from a terminal the owner holds the books open, closes them, and
//!       makes, lists and adopts books, without ever naming or touching
//!       the books' private folder.
//! SOURCE: plan/served.md 3.9 (a terminal running `wanderbooks serve`, and
//!         any command that needs the server, holds a lease; "Close the
//!         books" ends every lease), 3.1 (the CLI talks to admin.sock and
//!         never reads the books), 5.5 (adopt; preview and confirmation at
//!         the terminal).
//! INVARIANTS:
//!   1. A command that needs the server holds a lease while it runs, and
//!      lets go when it ends.
//!   2. Nothing printed names the data root.
//!   3. Adoption is previewed first and goes ahead only on a yes at the
//!      terminal, or --yes; a vault beside the books must be named (or
//!      --no-vault given): it is never silently left behind.
//!   4. A token made is kept in the keyring before anything is said of it,
//!      and its text is printed only by `token print`; one that can't be
//!      kept is revoked again. A token revoked is forgotten by the keyring
//!      too.
//! ```

use crate::knobs::{FLAG, Knobs};
use crate::lease::{Ask, HostMsg, Intent, Kind, Reason};
use crate::opener::{HoldError, Lease, Opener, Starter};
use ledger_mcp::admin::{self, Request, Response};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// How long a command waits for the books' host to answer one question.
const HOST_WAIT: Duration = Duration::from_secs(5);
/// How long a command waits for the books server to list the books.
const LIST_WAIT: Duration = Duration::from_secs(30);
/// How long `close` waits to see the books closed.
const CLOSE_WAIT: Duration = Duration::from_secs(120);

/// Common flags: `--root DIR` and the test knobs.
#[derive(Default)]
struct Common {
    root: Option<PathBuf>,
    knobs: Knobs,
}

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
/// keyring's test stand-in lives).
fn with_server_in(common: &Common, request: Request) -> Result<(Response, PathBuf), String> {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|e| e.to_string())?;
    let outcome = rt.block_on(async {
        let mut opener = Opener::new(
            common.root.as_deref(),
            common.knobs,
            Starter::Direct,
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
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        if common.take(&arg, &mut args)? {
            continue;
        }
        match arg.as_str() {
            "--help" | "-h" => {
                eprintln!("usage: {prog}");
                eprintln!("  Opens the books and holds them open until Ctrl-C. They close 10 s");
                eprintln!("  after nothing else (the app, an agent session) holds them.");
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
            Starter::Direct,
            Kind::Serve,
            None,
        )?;
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
    eprintln!("  list    the books, and whether each is open");
    eprintln!("  create  start new, empty books (base currency USD unless given)");
    eprintln!("  adopt   copy existing books in, verified; the original is left as it was");
}

/// `wanderbooks books list|create|adopt`.
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
                match b.problem {
                    None => println!("{}  open", b.id),
                    Some(why) => println!("{}  didn't open: {why}", b.id),
                }
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
                        match b.problem {
                            None => println!("  {}  open", b.id),
                            Some(why) => println!("  {}  didn't open: {why}", b.id),
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
    eprintln!("       {prog} print --book ID [--name NAME]");
    eprintln!("  create  make an agent's token for the books ID and keep it in this");
    eprintln!("          computer's keyring, where `wanderbooks mcp --book ID` finds it");
    eprintln!("          (NAME: agent unless given; LIST: read,write,post,admin unless");
    eprintln!("          given; --replace ends a token of the same name)");
    eprintln!("  list    the tokens there are (never their text)");
    eprintln!("  revoke  end a token, at once, and forget it here");
    eprintln!("  print   the token's text, for a client that sends it itself");
}

/// `wanderbooks token create|list|revoke|print`.
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
            let request = Request::TokenCreate {
                book: book.clone(),
                name: name.clone(),
                scopes,
                replace,
            };
            let (answer, run_dir) = with_server_in(&common, request).map_err(fail)?;
            let (info, text) = match answer {
                Response::TokenCreated { token, text } => (token, zeroize::Zeroizing::new(text.0)),
                Response::Refused(why) => return Err(fail(why)),
                other => return Err(fail(format!("the books server answered oddly: {other:?}"))),
            };
            // INVARIANT 4: kept first, or taken back.
            let root = run_dir.parent().unwrap_or(&run_dir).to_path_buf();
            let account = crate::tokens::Account::new(&root, &book, &name);
            let kept = crate::tokens::keeper(&run_dir, &common.knobs).set(&account, &text);
            drop(text);
            if let Err(why) = kept {
                let undone = with_server(
                    &common,
                    Request::TokenRevoke {
                        book: book.clone(),
                        name: name.clone(),
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
                return Err(fail(format!(
                    "the token was made, but this computer's keyring couldn't keep it ({why}), {after}.{replaced}"
                )));
            }
            println!(
                "Made the token {:?} for the books {} ({}), kept in this computer's keyring.",
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
                    let forgot = crate::tokens::keeper(&run_dir, &common.knobs).delete(&account);
                    println!("Revoked the token {:?} for the books {}.", t.name, t.book);
                    if let Err(why) = forgot {
                        eprintln!(
                            "warning: this computer's keyring still holds its text ({why}); it opens nothing now"
                        );
                    }
                    Ok(())
                }
                Response::Refused(why) => Err(fail(why)),
                other => Err(fail(format!("the books server answered oddly: {other:?}"))),
            }
        }
        ["print"] => {
            let book = need_book()?;
            let opener = Opener::new(
                common.root.as_deref(),
                common.knobs,
                Starter::Direct,
                Kind::Cli,
                None,
            )
            .map_err(|e| fail(e.to_string()))?;
            let run_dir = opener.run_dir().to_path_buf();
            let root = run_dir.parent().unwrap_or(&run_dir).to_path_buf();
            let account = crate::tokens::Account::new(&root, &book, &name);
            match crate::tokens::keeper(&run_dir, &common.knobs).get(&account) {
                Ok(Some(text)) => {
                    println!("{text}");
                    Ok(())
                }
                Ok(None) => Err(fail(format!(
                    "this computer keeps no token {name:?} for the books {book}: `wanderbooks token create --book {book}` makes one"
                ))),
                Err(why) => Err(fail(why)),
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

#[cfg(test)]
mod tests {
    use super::*;

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
            problem: None,
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
            ..adopted()
        });
        assert_eq!(
            all,
            [
                "Adopted as \"old\": 5 stored files copied, 4 of them checked against the books, 1 the books don't name (kept), 2 unfinished or hidden files left behind.",
                "TIN vault: 3 records and its key.",
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
