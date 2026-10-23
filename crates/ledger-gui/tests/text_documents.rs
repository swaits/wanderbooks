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

/// A PNG `w` by `h` pixels.
fn png(w: u32, h: u32) -> Vec<u8> {
    let img = image::RgbaImage::from_pixel(w, h, image::Rgba([40, 90, 160, 255]));
    let mut out = Vec::new();
    image::DynamicImage::ImageRgba8(img)
        .write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
        .unwrap();
    out
}

/// receipts.rs: every file's thumbnail is started when the books load,
/// before its page is opened; it is kept on disk as `<sha>.png`, at most
/// 560 pixels wide (a smaller picture is never scaled up), and the next
/// launch draws what was kept instead of the file.
#[test]
fn thumbnails_are_drawn_at_load_and_kept_by_hash() {
    let dir = TempDir::new("thumbs");
    let mut s = Store::create(&dir.0.join("books-data"), Currency::USD).unwrap();
    // The second of what filing gives is the file's SHA-256.
    let (_, big_sha, _) = s
        .file_document("photo", "wide.png", "image/png", &png(1200, 600), None)
        .unwrap();
    let (_, small_sha, _) = s
        .file_document("photo", "small.png", "image/png", &png(300, 200), None)
        .unwrap();
    drop(s);
    let thumbs = dir.0.join("thumbs");
    // The overview, not the documents page: the thumbnails come anyway.
    let mut h = open(&dir, Route::Overview);
    h.state_mut().keep_thumbs_in(thumbs.clone());
    let ctx = h.ctx.clone();
    h.state_mut().settle(&ctx);
    h.run_steps(2);
    assert_eq!(h.state().thumb_size(&big_sha), Some([560, 280]));
    assert_eq!(h.state().thumb_size(&small_sha), Some([300, 200]));
    let kept = thumbs.join(format!("{big_sha}.png"));
    let img = image::open(&kept).unwrap();
    assert_eq!((img.width(), img.height()), (560, 280));
    assert!(thumbs.join(format!("{small_sha}.png")).exists());
    // What was kept is what the next launch draws.
    std::fs::write(&kept, png(7, 5)).unwrap();
    let mut h = open(&dir, Route::Documents);
    h.state_mut().keep_thumbs_in(thumbs.clone());
    let ctx = h.ctx.clone();
    h.state_mut().settle(&ctx);
    assert_eq!(h.state().thumb_size(&big_sha), Some([7, 5]));
}

/// receipts.rs: a long picture's thumbnail is its top, at most 420
/// pixels tall; a picture past a texture's side opens scaled to fit
/// rather than crash the viewer; a file whose blob is gone shows as an
/// error even with its thumbnail kept.
#[test]
fn long_and_huge_pictures_are_cut_or_scaled_and_a_lost_file_is_an_error() {
    let dir = TempDir::new("long");
    let mut s = Store::create(&dir.0.join("books-data"), Currency::USD).unwrap();
    let (_, long_sha, _) = s
        .file_document("photo", "long.png", "image/png", &png(400, 9000), None)
        .unwrap();
    let (huge, huge_sha, _) = s
        .file_document("photo", "huge.png", "image/png", &png(9000, 300), None)
        .unwrap();
    drop(s);
    let thumbs = dir.0.join("thumbs");
    let mut h = open(&dir, Route::Document(huge));
    h.state_mut().keep_thumbs_in(thumbs.clone());
    let ctx = h.ctx.clone();
    h.state_mut().settle(&ctx);
    h.run_steps(2);
    assert_eq!(h.state().thumb_size(&long_sha), Some([400, 420]));
    assert_eq!(h.state().thumb_size(&huge_sha), Some([560, 18]));
    assert!(h.query_by_label_contains("Can't show").is_none());
    // The blob gone: the kept thumbnail is not shown in its place.
    let blob = walk(&dir.0.join("books-data"), &long_sha).expect("the blob's file");
    std::fs::remove_file(blob).unwrap();
    let mut h = open(&dir, Route::Documents);
    h.state_mut().keep_thumbs_in(thumbs);
    let ctx = h.ctx.clone();
    h.state_mut().settle(&ctx);
    assert_eq!(h.state().thumb_size(&long_sha), None);
}

/// The file under `dir` named `name`.
fn walk(dir: &std::path::Path, name: &str) -> Option<PathBuf> {
    for e in std::fs::read_dir(dir).ok()?.flatten() {
        let p = e.path();
        if p.is_dir() {
            if let Some(f) = walk(&p, name) {
                return Some(f);
            }
        } else if p.file_name().and_then(|n| n.to_str()) == Some(name) {
            return Some(p);
        }
    }
    None
}
