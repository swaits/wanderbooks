//! `ledger-gui`: a read-only, live view of wanderbooks books for the people
//! who own them and the accountant who checks them. See `plan/gui.md`.

#![allow(missing_docs)]

pub mod app;
pub mod attention;
pub mod books;
pub mod charts;
pub mod data;
pub mod demo;
pub mod export;
pub mod fmt;
pub mod menu;
pub mod nav;
pub mod pages;
pub mod palette;
mod prefs;
mod receipts;
mod schemes;
#[cfg(test)]
mod test_support;
mod theme;
mod widgets;

pub use app::App;
pub use data::Source;

use std::path::PathBuf;

/// The vault key file named on the command line (`--vault-key-file`),
/// which wins over a settings file's `[vault] key_file`, as it does for
/// the server: books that keep their own settings never name it.
static VAULT_KEY_FILE: std::sync::OnceLock<std::path::PathBuf> = std::sync::OnceLock::new();

/// Name the vault key file (once, at start).
pub fn set_vault_key_file(path: std::path::PathBuf) {
    let _ = VAULT_KEY_FILE.set(path);
}

/// The vault key file named on the command line, if one was.
pub fn vault_key_file() -> Option<std::path::PathBuf> {
    VAULT_KEY_FILE.get().cloned()
}

/// How [`run`] stops when it can't go on: the reason is already printed
/// on stderr, and the number is the process's exit code (2: a bad argument; 1: the demo books couldn't be built). A
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

/// Run the viewer with `args` (the command line after the program name);
/// `prog` names the program in the usage text (`ledger-gui`, or
/// `wanderbooks gui`). It opens the books read-only and returns when the
/// window closes. An unknown argument returns [`Exit`] (code 2).
///
/// `ledger-gui [--root PATH] [--book ID] [--data-dir PATH [--books-toml PATH] [--vault-key-file PATH]] [--today YYYY-MM-DD] [--demo]`:
/// a book of the data root (plan 3.6): `--root`, else the root `wanderbooks
/// setup` made, never the current directory or a settings file; `--book`
/// names one, else the root's only one (or the first, by id).
/// `--data-dir` opens an older books dir's `books-data` instead (its
/// settings file `--books-toml`, read while its journal keeps none, else
/// the `wanderbooks.toml` beside it; its vault key `--vault-key-file`);
/// `--demo` builds a demo company in a temp directory and opens that.
pub fn run(
    prog: &str,
    args: impl IntoIterator<Item = String>,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut root: Option<PathBuf> = None;
    let mut book: Option<String> = None;
    let mut data_dir: Option<PathBuf> = None;
    let mut today = None;
    let mut demo = false;
    let mut args = args.into_iter();
    let usage = || {
        format!(
            "usage: {prog} [--root PATH] [--book ID] [--data-dir PATH [--books-toml PATH] [--vault-key-file PATH]] [--today YYYY-MM-DD] [--demo]"
        )
    };
    while let Some(a) = args.next() {
        match a.as_str() {
            "--root" => root = args.next().map(PathBuf::from),
            "--book" => book = args.next(),
            "--data-dir" => data_dir = args.next().map(PathBuf::from),
            // An older books dir's settings file, read only while its
            // books keep no settings of their own (data::legacy_toml).
            "--books-toml" => {
                if let Some(t) = args.next() {
                    crate::data::set_legacy_toml(PathBuf::from(t));
                }
            }
            "--vault-key-file" => {
                if let Some(k) = args.next() {
                    crate::set_vault_key_file(PathBuf::from(k));
                }
            }
            "--today" => today = args.next().and_then(|d| d.parse().ok()),
            "--demo" => demo = true,
            "-h" | "--help" => {
                println!("{}", usage());
                return Ok(());
            }
            other => {
                eprintln!("unknown argument {other}");
                eprintln!("{}", usage());
                return Err(Box::new(Exit(2)));
            }
        }
    }
    if demo {
        let d = today.unwrap_or_else(|| chrono::Local::now().date_naive());
        let dir = std::env::temp_dir().join(format!("wanderbooks-demo-{d}"));
        let _ = std::fs::remove_dir_all(&dir);
        if let Err(e) = crate::demo::build(&dir, d) {
            eprintln!("building the demo books failed: {e}");
            return Err(Box::new(Exit(1)));
        }
        eprintln!("demo books: {}", dir.display());
        data_dir = Some(dir.join("books-data"));
    }
    let data_dir = match data_dir {
        Some(d) => d,
        None => match book_in_root(root.as_deref(), book.as_deref()) {
            Ok(d) => d,
            Err(why) => {
                eprintln!("{why}");
                return Err(Box::new(Exit(2)));
            }
        },
    };
    let source = Source { data_dir, today };
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("wanderbooks")
            .with_app_id("wanderbooks")
            .with_inner_size([1440.0, 920.0])
            // Small on purpose: tiling window managers hand out small slots
            // regardless, and every screen lays itself out to fit.
            .with_min_inner_size([360.0, 400.0]),
        // The platform's place for app settings, never the books' folder
        // (prefs.rs).
        persistence_path: prefs::prepare(),
        ..Default::default()
    };
    Ok(eframe::run_native(
        "wanderbooks",
        options,
        Box::new(move |cc| Ok(Box::new(App::new(cc, source)))),
    )?)
}

/// The folder of a book in the data root (plan 3.4): `root`, else the
/// one `wanderbooks setup` made; `book`, else its only book (or the first
/// by id). Why not, in words for the human.
pub fn book_in_root(root: Option<&std::path::Path>, book: Option<&str>) -> Result<PathBuf, String> {
    use ledger_store::root::{BookId, Root};
    let root = Root::find(root).map_err(|e| {
        format!("no books to show: {e}. `wanderbooks setup` makes a place for them.")
    })?;
    let books = root.list_books().map_err(|e| e.to_string())?;
    let id = match book {
        Some(b) => {
            let id = BookId::parse(b).map_err(|e| e.to_string())?;
            if !books.contains(&id) {
                return Err(format!(
                    "there are no books called {:?}: `wanderbooks books list` lists them",
                    id.as_str()
                ));
            }
            id
        }
        None => books.into_iter().next().ok_or_else(|| {
            "there are no books yet: your agent makes them (`wanderbooks books create`)."
                .to_string()
        })?,
    };
    Ok(root.book_dir(&id))
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
