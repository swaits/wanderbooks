//! `ledger-gui [--data-dir PATH] [--books-toml PATH] [--today YYYY-MM-DD] [--demo]`
//!
//! Opens the books read-only and follows them live. With no flags it looks
//! for `books-data/` and `wanderbooks.toml` in the current directory.
//! `--demo` builds a demo company in a temp directory and opens that.

use ledger_gui::{App, Source};
use std::path::PathBuf;

fn main() -> eframe::Result<()> {
    let mut data_dir: Option<PathBuf> = None;
    let mut books_toml: Option<PathBuf> = None;
    let mut today = None;
    let mut demo = false;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--data-dir" => data_dir = args.next().map(PathBuf::from),
            "--books-toml" => books_toml = args.next().map(PathBuf::from),
            "--today" => today = args.next().and_then(|d| d.parse().ok()),
            "--demo" => demo = true,
            "-h" | "--help" => {
                println!(
                    "usage: ledger-gui [--data-dir PATH] [--books-toml PATH] [--today YYYY-MM-DD] [--demo]"
                );
                return Ok(());
            }
            other => {
                eprintln!("unknown argument {other}");
                std::process::exit(2);
            }
        }
    }
    if demo {
        let d = today.unwrap_or_else(|| chrono::Local::now().date_naive());
        let dir = std::env::temp_dir().join(format!("wanderbooks-demo-{d}"));
        let _ = std::fs::remove_dir_all(&dir);
        if let Err(e) = ledger_gui::demo::build(&dir, d) {
            eprintln!("building the demo books failed: {e}");
            std::process::exit(1);
        }
        eprintln!("demo books: {}", dir.display());
        data_dir = Some(dir.join("books-data"));
        books_toml = Some(dir.join("wanderbooks.toml"));
    }
    let data_dir = data_dir.unwrap_or_else(|| PathBuf::from("books-data"));
    let books_toml = books_toml.or_else(|| {
        let guess = data_dir
            .parent()
            .unwrap_or(&data_dir)
            .join("wanderbooks.toml");
        guess.exists().then_some(guess)
    });
    let source = Source {
        data_dir,
        books_toml,
        today,
    };
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("wanderbooks")
            .with_app_id("wanderbooks")
            .with_inner_size([1440.0, 920.0])
            .with_min_inner_size([900.0, 600.0]),
        ..Default::default()
    };
    eframe::run_native(
        "wanderbooks",
        options,
        Box::new(move |cc| Ok(Box::new(App::new(cc, source)))),
    )
}
