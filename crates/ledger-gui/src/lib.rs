//! `ledger-gui`: a read-only, live view of wanderbooks books for the people
//! who own them and the accountant who checks them. See `plan/gui.md`.

#![allow(missing_docs)]

pub mod app;
pub mod attention;
pub mod books;
pub mod charts;
pub mod data;
pub mod demo;
pub mod dock;
pub mod export;
pub mod fmt;
pub mod menu;
pub mod nav;
pub mod owner;
pub mod pages;
pub mod palette;
mod prefs;
mod receipts;
mod schemes;
#[cfg(test)]
mod test_support;
mod theme;
pub mod welcome;
mod widgets;

pub use app::App;
pub use data::Source;

use std::path::PathBuf;

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
/// `ledger-gui [--root PATH] [--book ID] [--today YYYY-MM-DD] [--demo]`:
/// a book of the data root (plan 3.6), the only books it opens (D-44):
/// `--root`, else the root `wanderbooks setup` made, never the current
/// directory or a settings file; `--book` names one, else the root's only
/// one (or the first, by id). With no books yet, and an owner to set them
/// up (`wanderbooks gui`), it opens the first-run page (welcome.rs) instead
/// of ending. `--demo` builds a demo company in a temp directory and opens
/// that.
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
    let usage = || format!("usage: {prog} [--root PATH] [--book ID] [--today YYYY-MM-DD] [--demo]");
    while let Some(a) = args.next() {
        match a.as_str() {
            "--root" => root = args.next().map(PathBuf::from),
            "--book" => book = args.next(),
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
    // The books to show, or a first run (no books yet, and the program gave
    // the viewer an owner to set them up: plan P11b), or the reason not.
    let mut first_run = None;
    let data_dir = match data_dir {
        Some(d) => Some(d),
        None => match book_in_root(root.as_deref(), book.as_deref()) {
            Ok(d) => Some(d),
            Err(why) => match owner::current() {
                // Named books that aren't there are a mistake, not a first run.
                Some(owner) if book.is_none() => {
                    first_run = Some(owner.first_run());
                    None
                }
                _ => {
                    eprintln!("{why}");
                    return Err(Box::new(Exit(2)));
                }
            },
        },
    };
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
        Box::new(move |cc| {
            Ok(Box::new(match (data_dir, first_run) {
                (Some(data_dir), _) => App::new(cc, Source { data_dir, today }),
                (None, state) => App::first_run(cc, root, today, state.unwrap_or_default()),
            }))
        }),
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
