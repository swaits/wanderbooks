//! `wanderbooks`: the one executable (plan D-22). Its modes are the
//! viewer (`gui`), the books server (`server`, and `dev-stdio` over stdio),
//! the host and the agents' bridge (`host`, `mcp`), and the owner's
//! commands (`setup`, `books`, `connect`, `token`, `passphrase`, `serve`,
//! `close`, `doctor`, `upload`, `download`). A build without the `gui`
//! feature (the server-only build for another machine) has no viewer.
//!
//! From nothing to the first post: `wanderbooks setup`, `wanderbooks books
//! create ID`, `wanderbooks connect claude --book ID` (then talk to the
//! agent).

use std::env;
use std::path::PathBuf;

fn print_usage() {
    eprintln!("usage: wanderbooks setup [--root PATH] [--home DIR]");
    eprintln!("       wanderbooks gui [BOOK | --root PATH --book ID | --data-dir PATH ...]");
    eprintln!(
        "       wanderbooks books list | create ID | adopt DATA-DIR --id ID | restore BACKUP | back-up ID | upgrade ID | put-aside ID DATA-DIR [PATH...]"
    );
    eprintln!("       wanderbooks connect claude|codex --book ID");
    eprintln!(
        "       wanderbooks token create --book ID | list | revoke --book ID | print --book ID"
    );
    eprintln!("       wanderbooks mcp --book ID [--token NAME]");
    eprintln!("       wanderbooks passphrase");
    eprintln!("       wanderbooks upload --book ID [--token NAME] UPLOAD [--] FILE");
    eprintln!("       wanderbooks download --book ID [--token NAME] ATTACHMENT --to FOLDER");
    eprintln!("       wanderbooks serve | close | doctor");
    eprintln!("       wanderbooks dev-stdio [--data-dir PATH ...]");
    eprintln!();
    eprintln!("  setup      make the books' private folder (only your account can open it)");
    eprintln!("  gui        open the live, read-only view of the books (a book of the data root)");
    eprintln!(
        "  books      list, start, adopt, restore, back up or upgrade books (`books --help`)"
    );
    eprintln!("  connect    let Claude Code or Codex reach the books");
    eprintln!("  token      make, list or revoke an agent's token for books (`token --help`)");
    eprintln!("  mcp        an agent's way to the books, over stdio (what `connect` registers)");
    eprintln!("  passphrase set or change the owner passphrase (typed at a terminal)");
    eprintln!("  upload     send a file for an upload an agent began (books.begin_upload)");
    eprintln!("  download   save a copy of a stored file (one too large to come back inline)");
    eprintln!("  serve      hold the books open until Ctrl-C");
    eprintln!("  close      close the books now: everything holding them lets go");
    eprintln!("  doctor     whether the books are open, what holds them, what went wrong");
    eprintln!("  dev-stdio  run the books server over stdio (`dev-stdio --help` for its flags)");
}

/// The viewer's arguments for `wanderbooks gui ARGS` (plan 3.6): a lone
/// word names the book of the data root to show (`--book`), never a
/// folder; nothing, the root's book; anything else is the viewer's own
/// flags, passed as they are.
#[cfg(feature = "gui")]
fn gui_args(args: Vec<String>) -> Vec<String> {
    match args.as_slice() {
        [book] if !book.starts_with('-') => vec!["--book".into(), book.clone()],
        _ => args,
    }
}

/// The viewer's owner (ledger-gui owner.rs): its Books menu, done as the
/// command line does it (the books server, the agents' own commands).
#[cfg(all(feature = "gui", unix))]
struct ViewerOwner(ledger_host::commands::Common);

#[cfg(all(feature = "gui", unix))]
impl ledger_gui::owner::Owner for ViewerOwner {
    fn close_books(&self) -> Result<String, String> {
        ledger_host::commands::close_books(&self.0)
    }

    fn new_books(&self, id: &str, currency: &str) -> Result<String, String> {
        ledger_host::commands::new_books(&self.0, id, currency)
    }

    fn back_up_now(&self, book: &str) -> Result<Vec<String>, String> {
        ledger_host::commands::back_up_now(&self.0, book, false)
    }

    fn plan_restore(
        &self,
        source: &str,
        id: Option<String>,
        replace: bool,
    ) -> Result<ledger_gui::owner::RestorePlan, String> {
        let p = ledger_host::commands::plan_restore(&self.0, source, id, replace, false)?;
        Ok(ledger_gui::owner::RestorePlan {
            source: p.source,
            id: p.id,
            replace: p.replace,
            head: p.head,
            lines: p.lines,
        })
    }

    fn restore(&self, plan: &ledger_gui::owner::RestorePlan) -> Result<Vec<String>, String> {
        ledger_host::commands::carry_out_restore(
            &self.0,
            &ledger_host::commands::RestorePlan {
                source: plan.source.clone(),
                id: plan.id.clone(),
                replace: plan.replace,
                head: plan.head.clone(),
                origin: None,
                lines: plan.lines.clone(),
            },
        )
    }

    fn connect(&self, agent: ledger_gui::owner::Agent, book: &str) -> Result<Vec<String>, String> {
        let agent = match agent {
            ledger_gui::owner::Agent::Claude => "claude",
            ledger_gui::owner::Agent::Codex => "codex",
        };
        ledger_host::commands::connect_agent(&self.0, agent, book)
    }

    fn upgrade_format(&self, book: &str, passphrase: &str) -> Result<Vec<String>, String> {
        ledger_host::commands::upgrade_format(&self.0, book, passphrase)
    }
}

/// `wanderbooks gui`: the viewer, in this process. For books of the data
/// root it holds them open while it runs, and its Books menu acts on them.
#[cfg(feature = "gui")]
fn run_gui(args: Vec<String>) -> i32 {
    let args = gui_args(args);
    #[cfg(unix)]
    {
        let value = |flag: &str| {
            // The last given, as the viewer reads it.
            args.iter()
                .rposition(|a| a == flag)
                .and_then(|i| args.get(i + 1))
                .cloned()
        };
        // Only the viewer's own flags for books of the data root, each with
        // its value: anything else (help, a mistake, an older books dir,
        // the demo) holds nothing.
        let mut known = true;
        let mut rest = args.iter();
        while let Some(a) = rest.next() {
            if !matches!(a.as_str(), "--root" | "--book" | "--today") || rest.next().is_none() {
                known = false;
                break;
            }
        }
        let root = value("--root").map(PathBuf::from);
        let book = value("--book");
        if known && ledger_gui::book_in_root(root.as_deref(), book.as_deref()).is_ok() {
            // The viewer asks about a format upgrade in its own window.
            let common = ledger_host::commands::Common {
                root,
                ..Default::default()
            };
            ledger_gui::owner::set_owner(std::sync::Arc::new(ViewerOwner(common.clone())));
            ledger_host::commands::hold_for_viewer(&common, book);
        }
    }
    match ledger_gui::run("wanderbooks gui", args) {
        Ok(()) => 0,
        Err(e) => match e.downcast_ref::<ledger_gui::Exit>() {
            Some(exit) => exit.0,
            None => {
                eprintln!("error: {e}");
                1
            }
        },
    }
}

/// `wanderbooks gui` in a build without the viewer.
#[cfg(not(feature = "gui"))]
fn run_gui(_args: Vec<String>) -> i32 {
    eprintln!("error: this wanderbooks is the server-only build: it has no viewer");
    2
}

/// `wanderbooks setup [--root PATH] [--home DIR]`: make the books'
/// private folder (plan 3.4): `--root` if given, else
/// `$XDG_DATA_HOME/wanderbooks`, else the platform's default under the
/// account's home (`--home`, else the system's user records). The folder's
/// location is never printed: nothing an agent is shown names it.
fn run_setup(args: Vec<String>) -> i32 {
    let mut root: Option<PathBuf> = None;
    let mut home: Option<PathBuf> = None;
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        let slot = match arg.as_str() {
            "--root" => &mut root,
            "--home" => &mut home,
            "--help" | "-h" => {
                eprintln!("usage: wanderbooks setup [--root PATH] [--home DIR]");
                eprintln!(
                    "  --root  where the books' private folder goes (default: your data folder)"
                );
                eprintln!("  --home  the account's home (default: from the system's user records)");
                return 0;
            }
            other => {
                eprintln!("unknown argument: {other}");
                eprintln!("usage: wanderbooks setup [--root PATH] [--home DIR]");
                return 2;
            }
        };
        match args.next() {
            Some(value) => *slot = Some(PathBuf::from(value)),
            None => {
                eprintln!("error: {arg} needs a path");
                return 1;
            }
        }
    }
    let account = ledger_store::root::account_home().ok();
    let home = match home.or_else(|| account.clone()) {
        Some(home) => home,
        None => {
            eprintln!("error: {}", ledger_store::root::RootError::NoHome);
            return 1;
        }
    };
    if account.as_ref() != Some(&home) {
        eprintln!(
            "note: --home isn't this account's home, so this account's books server won't \
             look there (tests and unusual setups only)"
        );
    }
    // XDG_DATA_HOME is this account's, for this account's home: with
    // another home given (tests), it is never read, so it can't aim at
    // this account's real books.
    let xdg = if account.as_ref() == Some(&home) {
        env::var_os("XDG_DATA_HOME").map(PathBuf::from)
    } else {
        None
    };
    match ledger_store::root::Root::setup(&home, root.as_deref(), xdg.as_deref()) {
        Ok((found, report)) => {
            let what = if report.created {
                "made"
            } else if report.tightened {
                "already there; made private again (other users could open it)"
            } else {
                "already there"
            };
            println!("The books' private folder: {what}. Only your account can open it.");
            if report.recorded {
                println!("It isn't in the default place, so where it is was noted for next time.");
            }
            owner_passphrase(&found)
        }
        Err(e) => {
            eprintln!("error: {e}");
            1
        }
    }
}

/// After setup: the owner passphrase, asked for at a terminal if none is
/// set yet; elsewhere (a script, an agent) only said how to set it.
#[cfg(unix)]
fn owner_passphrase(root: &ledger_store::root::Root) -> i32 {
    let auth = match root.auth_dir() {
        Ok(dir) => dir,
        Err(e) => {
            eprintln!("error: {e}");
            return 1;
        }
    };
    match ledger_store::passphrase::is_set(&auth) {
        Ok(true) => return 0,
        Ok(false) => {}
        Err(e) => {
            eprintln!("error: {e}");
            return 1;
        }
    }
    if !std::io::IsTerminal::is_terminal(&std::io::stdin()) {
        println!("No owner passphrase yet: set one in a terminal with `wanderbooks passphrase`.");
        return 0;
    }
    println!(
        "Now the owner passphrase: what the books ask you for before anything only you may do."
    );
    match ledger_host::commands::set_passphrase_at_terminal(&auth) {
        Ok(()) => {
            println!("The owner passphrase is set.");
            0
        }
        Err(why) => {
            eprintln!(
                "error: {why}. The folder is made; set the passphrase with `wanderbooks passphrase`."
            );
            1
        }
    }
}

#[cfg(not(unix))]
fn owner_passphrase(_root: &ledger_store::root::Root) -> i32 {
    0
}

/// `wanderbooks dev-stdio`: the books server over stdio, in this process.
fn run_dev_stdio(args: Vec<String>) -> i32 {
    match ledger_mcp::run("wanderbooks dev-stdio", args) {
        Ok(()) => 0,
        Err(e) => match e.downcast_ref::<ledger_mcp::Exit>() {
            Some(exit) => exit.0,
            None => {
                eprintln!("error: {e}");
                1
            }
        },
    }
}

/// `wanderbooks server`: the books server for the data root, on its
/// local sockets (plan 3.9). The host starts it.
#[cfg(unix)]
fn run_server(args: Vec<String>) -> i32 {
    match ledger_mcp::local::run("wanderbooks server", args) {
        Ok(()) => 0,
        Err(e) => match e.downcast_ref::<ledger_mcp::Exit>() {
            Some(exit) => exit.0,
            None => {
                eprintln!("error: {e}");
                1
            }
        },
    }
}

/// The modes that open and hold the books (plan 3.9): the host, the
/// agents' bridge, and the owner's commands.
#[cfg(unix)]
fn run_local(mode: &str, args: Vec<String>) -> i32 {
    let prog = format!("wanderbooks {mode}");
    let result = match mode {
        "host" => ledger_host::host::run(&prog, args),
        "mcp" => ledger_host::bridge::run(&prog, args),
        "serve" => ledger_host::commands::serve(&prog, args),
        "close" => ledger_host::commands::close(&prog, args),
        "doctor" => ledger_host::commands::doctor(&prog, args),
        "token" => ledger_host::commands::token(&prog, args),
        "connect" => ledger_host::commands::connect(&prog, args),
        "passphrase" => ledger_host::commands::passphrase(&prog, args),
        _ => ledger_host::commands::books(&prog, args),
    };
    match result {
        Ok(()) => 0,
        Err(e) => match e.downcast_ref::<ledger_host::Exit>() {
            Some(exit) => exit.0,
            None => {
                eprintln!("error: {e}");
                1
            }
        },
    }
}

/// `upload` and `download`: the agent's file commands, whose words may
/// be paths that aren't text.
#[cfg(unix)]
fn run_files(mode: &str, args: Vec<std::ffi::OsString>) -> i32 {
    let prog = format!("wanderbooks {mode}");
    let result = match mode {
        "upload" => ledger_host::commands::upload(&prog, args),
        _ => ledger_host::commands::download(&prog, args),
    };
    match result {
        Ok(()) => 0,
        Err(e) => match e.downcast_ref::<ledger_host::Exit>() {
            Some(exit) => exit.0,
            None => {
                eprintln!("error: {e}");
                1
            }
        },
    }
}

/// The file commands, where local sockets aren't built yet.
#[cfg(not(unix))]
fn run_files(_mode: &str, _args: Vec<std::ffi::OsString>) -> i32 {
    eprintln!("error: opening the books this way comes to this system with P11");
    2
}

/// The modes that hold the books, where local sockets aren't built yet.
#[cfg(not(unix))]
fn run_local(_mode: &str, _args: Vec<String>) -> i32 {
    eprintln!("error: opening the books this way comes to this system with P11");
    2
}

/// `wanderbooks server` where local sockets aren't built yet.
#[cfg(not(unix))]
fn run_server(_args: Vec<String>) -> i32 {
    eprintln!("error: the books server's local sockets come to this system with P11");
    2
}

fn main() {
    // A file's path may be any name the system takes: the file commands
    // get the words as given. Everything else is text.
    let given: Vec<std::ffi::OsString> = env::args_os().skip(1).collect();
    if let Some(mode @ ("upload" | "download")) = given.first().and_then(|m| m.to_str()) {
        let mode = mode.to_string();
        std::process::exit(run_files(&mode, given[1..].to_vec()));
    }
    let Ok(words) = given
        .into_iter()
        .map(std::ffi::OsString::into_string)
        .collect::<Result<Vec<String>, _>>()
    else {
        eprintln!("error: an argument isn't UTF-8 text");
        std::process::exit(2);
    };
    // The modes run in this process and take their own flags.
    let mut all = words.clone().into_iter();
    match all.next().as_deref() {
        Some("gui") => std::process::exit(run_gui(all.collect())),
        Some("dev-stdio") => std::process::exit(run_dev_stdio(all.collect())),
        Some("setup") => std::process::exit(run_setup(all.collect())),
        Some("server") => std::process::exit(run_server(all.collect())),
        Some(
            mode @ ("host" | "mcp" | "serve" | "close" | "books" | "doctor" | "token" | "connect"
            | "passphrase"),
        ) => {
            let mode = mode.to_string();
            std::process::exit(run_local(&mode, all.collect()))
        }
        _ => {}
    }
    if words.first().is_some_and(|w| w == "--help" || w == "-h") {
        print_usage();
        return;
    }
    // The books-folder scaffold is gone (P2 slice 8): said plainly.
    if let Some(gone @ ("new" | "init")) = words.first().map(String::as_str) {
        eprintln!(
            "`wanderbooks {gone}` is gone: books now live in a private folder. Start them with `wanderbooks setup`, `wanderbooks books create ID` and `wanderbooks connect claude --book ID`."
        );
        std::process::exit(2);
    }
    // A mode named after a flag: it must come first.
    match words
        .iter()
        .map(String::as_str)
        .find(|w| !w.starts_with('-'))
    {
        Some(
            mode @ ("gui" | "dev-stdio" | "setup" | "server" | "host" | "mcp" | "serve" | "connect"
            | "close" | "books" | "doctor" | "token" | "passphrase" | "upload" | "download"),
        ) => {
            eprintln!("`{mode}` must come first: wanderbooks {mode} [its own flags]");
            std::process::exit(2);
        }
        _ => {
            print_usage();
            std::process::exit(2);
        }
    }
}

#[cfg(all(test, feature = "gui"))]
mod gui_args_tests {
    use super::gui_args;

    fn v(args: &[&str]) -> Vec<String> {
        args.iter().map(|a| a.to_string()).collect()
    }

    #[test]
    fn a_lone_word_names_a_book_never_a_folder() {
        assert_eq!(gui_args(v(&["acme"])), v(&["--book", "acme"]));
        assert_eq!(gui_args(v(&["."])), v(&["--book", "."]));
        assert_eq!(gui_args(v(&[])), v(&[]));
    }

    #[test]
    fn flags_and_anything_else_pass_as_they_are() {
        for args in [
            v(&["--demo"]),
            v(&["--data-dir", "x"]),
            v(&["a", "b"]),
            v(&["--help"]),
        ] {
            assert_eq!(gui_args(args.clone()), args);
        }
    }
}
