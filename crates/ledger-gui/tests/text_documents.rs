//! Text documents in the viewer: a Markdown file is drawn as formatted
//! text, a plain-text file as its text, both on the document's page and
//! on its tile; a text file that isn't UTF-8 says so rather than show
//! half of it.

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use ledger_core::Currency;
use ledger_gui::nav::Route;
use ledger_gui::{App, Source, data};
use ledger_store::Store;
use std::path::PathBuf;

struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> TempDir {
        let d = std::env::temp_dir().join(format!("ledger-gui-text-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        TempDir(d)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

const PLAN: &str = "\u{feff}# Studio plan\n\nThe studio's goal is **ten apps** that earn.\n\n\
- Chess openings\n- Route planner\n";

fn open(dir: &TempDir, route: Route) -> Harness<'static, App> {
    let books = data::load_once(&Source {
        data_dir: dir.0.join("books-data"),
        today: Some(chrono::NaiveDate::from_ymd_opt(2026, 10, 23).unwrap()),
    })
    .unwrap();
    let mut h = Harness::builder()
        .with_size(egui::vec2(1100.0, 900.0))
        .build_eframe(|cc| App::with_books(&cc.egui_ctx, books, false, route));
    h.run_steps(3);
    let ctx = h.ctx.clone();
    h.state_mut().settle(&ctx);
    h.run_steps(3);
    h
}

/// SHOT_DIR set: the page as drawn, to look at.
fn shot(h: &mut Harness<'static, App>, name: &str) {
    if let Ok(out) = std::env::var("SHOT_DIR") {
        let out = PathBuf::from(out);
        std::fs::create_dir_all(&out).unwrap();
        h.render()
            .unwrap()
            .save(out.join(format!("{name}.png")))
            .unwrap();
    }
}

#[test]
fn a_markdown_document_is_drawn_as_formatted_text() {
    let dir = TempDir::new("md");
    let mut s = Store::create(&dir.0.join("books-data"), Currency::USD).unwrap();
    let (md, _, _) = s
        .file_document(
            "other",
            "plan.md",
            "text/markdown",
            PLAN.as_bytes(),
            Some("Plan for the studio"),
        )
        .unwrap();
    drop(s);
    let mut h = open(&dir, Route::Document(md));
    shot(&mut h, "markdown-document");
    // The heading is its own text, without the "#" or the byte-order mark;
    // the list items are there; nothing says it can't be shown.
    assert!(h.query_by_label("Studio plan").is_some());
    assert!(h.query_by_label_contains("# Studio").is_none());
    assert!(h.query_by_label_contains("Chess openings").is_some());
    assert!(h.query_by_label_contains("Route planner").is_some());
    assert!(h.query_by_label_contains("can't be previewed").is_none());
    assert!(h.query_by_label_contains("Can't show").is_none());
}

#[test]
fn a_plain_text_document_is_its_text_and_bad_text_says_so() {
    let dir = TempDir::new("txt");
    let mut s = Store::create(&dir.0.join("books-data"), Currency::USD).unwrap();
    let (txt, _, _) = s
        .file_document(
            "other",
            "notes.txt",
            "text/plain; charset=utf-8",
            b"Meeting notes\nall agreed",
            None,
        )
        .unwrap();
    let (bad, _, _) = s
        .file_document("other", "bad.md", "text/markdown", b"caf\xe9", None)
        .unwrap();
    drop(s);
    let h = open(&dir, Route::Document(txt));
    assert!(h.query_by_label_contains("Meeting notes").is_some());
    assert!(h.query_by_label_contains("Can't show").is_none());
    let h = open(&dir, Route::Document(bad));
    assert!(h.query_by_label_contains("the text isn't UTF-8").is_some());
}

#[test]
fn the_documents_page_draws_text_tiles() {
    let dir = TempDir::new("tiles");
    let mut s = Store::create(&dir.0.join("books-data"), Currency::USD).unwrap();
    s.file_document("other", "plan.md", "text/markdown", PLAN.as_bytes(), None)
        .unwrap();
    drop(s);
    let mut h = open(&dir, Route::Documents);
    h.run_steps(2);
    shot(&mut h, "text-tiles");
    assert_eq!(h.state_mut().route(), &Route::Documents);
}
