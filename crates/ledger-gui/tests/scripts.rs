//! Names in other scripts show as written (P3 review round 2): the viewer
//! falls back on the PDFs' own Noto fonts (`ledger_import::fonts`), so an
//! account named in Chinese, Arabic, Hebrew, Thai or Hindi is drawn with
//! its own letters, never as empty boxes.
//!
//! ```text
//! GOAL: the owner reads every name the books hold, in any script.
//! SOURCE: DECISION (lead, P3 review round 2, 2026-10-20).
//! ```
//!
//! Set `SHOT_DIR` to also save the Accounts page as `scripts.png` there,
//! to look at.

use egui_kittest::Harness;
use ledger_core::{AccountType, Currency, Money, Side};
use ledger_gui::nav::Route;
use ledger_gui::{App, Source, data};
use ledger_store::Store;
use rust_decimal::Decimal;
use std::path::PathBuf;

/// A test's dir, removed when the test ends, pass or panic.
struct TestDir(PathBuf);

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// The names, each its own script.
const NAMES: [&str; 5] = ["北京 办公室", "مكتب الرياض", "שכר דירה", "ค่าเช่า", "किराया"];

#[test]
fn account_names_in_other_scripts_show_as_written() {
    let dir =
        TestDir(std::env::temp_dir().join(format!("ledger-gui-scripts-{}", std::process::id())));
    let _ = std::fs::remove_dir_all(&dir.0);
    let data_dir = dir.0.join("books-data");
    let mut s = Store::create(&data_dir, Currency::USD).unwrap();
    let cash = s
        .add_account("1000", "Checking", AccountType::Asset, None, false)
        .unwrap();
    // Each account with a payment on it, so the page lists it.
    for (i, name) in NAMES.iter().enumerate() {
        let id = s
            .add_account(&format!("6{i}00"), name, AccountType::Expense, None, false)
            .unwrap();
        let day = chrono::NaiveDate::from_ymd_opt(2026, 10, 1 + i as u32).unwrap();
        let e = s.create_entry(day, name).unwrap();
        let m = Money::new(Decimal::new(10_000 * (i as i64 + 1), 2), Currency::USD);
        s.add_line(e, id, Side::Debit, m).unwrap();
        s.add_line(e, cash, Side::Credit, m).unwrap();
        s.attach_receipt(Some(e), "r.pdf", "application/pdf", b"%PDF r", None)
            .unwrap();
        s.post_entry(e).unwrap();
    }
    drop(s);
    let books = data::load_once(&Source {
        data_dir,
        today: Some(chrono::NaiveDate::from_ymd_opt(2026, 10, 20).unwrap()),
    })
    .unwrap();
    let mut h = Harness::builder()
        .with_size(egui::vec2(1100.0, 640.0))
        .build_eframe(|cc| App::with_books(&cc.egui_ctx, books, false, Route::Accounts));
    h.run_steps(4);
    let ctx = h.ctx.clone();
    h.state_mut().settle(&ctx);
    h.run_steps(4);
    if let Ok(out) = std::env::var("SHOT_DIR") {
        let out = PathBuf::from(out);
        std::fs::create_dir_all(&out).unwrap();
        h.render().unwrap().save(out.join("scripts.png")).unwrap();
    }
    // Every text the page painted.
    fn walk(s: &egui::Shape, out: &mut Vec<String>) {
        match s {
            egui::Shape::Text(t) => out.push(t.galley.text().to_string()),
            egui::Shape::Vec(v) => v.iter().for_each(|s| walk(s, out)),
            _ => {}
        }
    }
    let mut painted = Vec::new();
    for c in &h.output().shapes {
        walk(&c.shape, &mut painted);
    }
    for name in NAMES {
        assert!(
            painted.iter().any(|t| t.contains(name)),
            "{name} is on the Accounts page: {painted:?}"
        );
        for c in name.chars().filter(|c| !c.is_whitespace()) {
            assert!(
                ctx.fonts_mut(|f| f.has_glyph(&egui::FontId::proportional(14.0), c)),
                "{c} of {name} has a glyph"
            );
        }
    }
}
