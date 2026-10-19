//! Test support: books built through the store, the viewer on one page,
//! and what that page painted (text, place, color), for tests that check
//! what a page says, not only that it renders.

use crate::app::{App, PageState};
use crate::books::Books;
use crate::nav::Route;
use crate::{Source, data};
use chrono::NaiveDate;
use egui::{Color32, Pos2};
use egui_kittest::Harness;
use ledger_core::{AccountId, AccountType, Currency, Money, Side};
use ledger_store::Store;
use rust_decimal::Decimal;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

pub(crate) fn d(y: i32, m: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, day).unwrap()
}

/// Dollars and cents as cents.
pub(crate) fn cents(c: i64) -> Decimal {
    Decimal::new(c, 2)
}

/// The day the test books are viewed on.
pub(crate) fn today() -> NaiveDate {
    d(2026, 10, 15)
}

/// A scratch directory, removed when dropped.
pub(crate) struct TempDir(pub PathBuf);

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

static DIRS: AtomicUsize = AtomicUsize::new(0);

/// A store to build test books in, and the accounts most tests use.
pub(crate) struct Build {
    pub dir: TempDir,
    pub s: Store,
    pub cash: AccountId,
    pub capital: AccountId,
    pub rent: AccountId,
}

impl Build {
    pub(crate) fn new(tag: &str) -> Build {
        let n = DIRS.fetch_add(1, Ordering::Relaxed);
        let dir = TempDir(
            std::env::temp_dir().join(format!("ledger-gui-unit-{tag}-{}-{n}", std::process::id())),
        );
        let _ = std::fs::remove_dir_all(&dir.0);
        let mut s = Store::create(&dir.0.join("books-data"), Currency::USD).unwrap();
        let mut add =
            |code: &str, name: &str, ty| s.add_account(code, name, ty, None, false).unwrap();
        let cash = add("1000", "Cash", AccountType::Asset);
        let capital = add("3000", "Owner Capital", AccountType::Equity);
        let rent = add("6000", "Rent", AccountType::Expense);
        Build {
            dir,
            s,
            cash,
            capital,
            rent,
        }
    }

    /// Post Dr `dr` / Cr `cr` `c` cents on `date`, with a receipt.
    pub(crate) fn post(&mut self, date: NaiveDate, dr: AccountId, cr: AccountId, c: i64) -> u64 {
        self.post_as("test", date, dr, cr, c)
    }

    /// A draft, Dr `dr` / Cr `cr` `c` cents on `date`, described `what`.
    pub(crate) fn draft(
        &mut self,
        what: &str,
        date: NaiveDate,
        dr: AccountId,
        cr: AccountId,
        c: i64,
    ) -> u64 {
        let usd = Money::new(cents(c), Currency::USD);
        let e = self.s.create_entry(date, what).unwrap();
        self.s.add_line(e, dr, Side::Debit, usd).unwrap();
        self.s.add_line(e, cr, Side::Credit, usd).unwrap();
        e
    }

    /// [`Self::post`], described `what`.
    pub(crate) fn post_as(
        &mut self,
        what: &str,
        date: NaiveDate,
        dr: AccountId,
        cr: AccountId,
        c: i64,
    ) -> u64 {
        let usd = Money::new(cents(c), Currency::USD);
        let e = self.s.create_entry(date, what).unwrap();
        self.s.add_line(e, dr, Side::Debit, usd).unwrap();
        self.s.add_line(e, cr, Side::Credit, usd).unwrap();
        self.s
            .attach_receipt(Some(e), "r.pdf", "application/pdf", b"%PDF r", None)
            .unwrap();
        self.s.post_entry(e).unwrap();
        e
    }

    /// The books as the viewer loads them, on [`today`].
    pub(crate) fn books(self) -> (TempDir, Books) {
        let data_dir = self.dir.0.join("books-data");
        drop(self.s);
        let books = data::load_once(&Source {
            data_dir,
            today: Some(today()),
        })
        .unwrap();
        (self.dir, books)
    }
}

/// One text a frame painted.
#[derive(Clone, Debug)]
pub(crate) struct Painted {
    pub text: String,
    pub pos: Pos2,
    pub width: f32,
    pub height: f32,
    pub color: Color32,
}

impl Painted {
    /// The middle of its line.
    pub(crate) fn mid_y(&self) -> f32 {
        self.pos.y + self.height / 2.0
    }
}

/// Every filled rectangle a frame painted, with its fill.
pub(crate) fn rects(out: &egui::FullOutput) -> Vec<(egui::Rect, Color32)> {
    fn walk(s: &egui::Shape, out: &mut Vec<(egui::Rect, Color32)>) {
        match s {
            egui::Shape::Rect(r) => out.push((r.rect, r.fill)),
            egui::Shape::Vec(v) => v.iter().for_each(|s| walk(s, out)),
            _ => {}
        }
    }
    let mut all = Vec::new();
    for c in &out.shapes {
        walk(&c.shape, &mut all);
    }
    all
}

/// Every text a frame painted.
pub(crate) fn painted(out: &egui::FullOutput) -> Vec<Painted> {
    fn walk(s: &egui::Shape, out: &mut Vec<Painted>) {
        match s {
            egui::Shape::Text(t) => {
                let color = t
                    .override_text_color
                    .or_else(|| t.galley.job.sections.first().map(|s| s.format.color))
                    .unwrap_or(t.fallback_color);
                out.push(Painted {
                    text: t.galley.text().to_string(),
                    pos: t.pos,
                    width: t.galley.size().x,
                    height: t.galley.size().y,
                    color,
                });
            }
            egui::Shape::Vec(v) => v.iter().for_each(|s| walk(s, out)),
            _ => {}
        }
    }
    let mut texts = Vec::new();
    for c in &out.shapes {
        walk(&c.shape, &mut texts);
    }
    texts
}

/// The viewer on `route`, with page state set by `set`, after it has
/// settled: what it painted, and its palette.
pub(crate) fn render(
    books: Books,
    route: Route,
    set: impl FnOnce(&mut PageState),
) -> (Vec<Painted>, crate::theme::Palette) {
    let (texts, _, p) = render_all(books, route, set);
    (texts, p)
}

/// [`render`], with the filled rectangles too.
pub(crate) fn render_all(
    books: Books,
    route: Route,
    set: impl FnOnce(&mut PageState),
) -> (
    Vec<Painted>,
    Vec<(egui::Rect, Color32)>,
    crate::theme::Palette,
) {
    let mut h = Harness::builder()
        .with_size(egui::vec2(1400.0, 2400.0))
        .build_eframe(|cc| App::with_books(&cc.egui_ctx, books, false, route));
    set(h.state_mut().state_mut());
    h.run_steps(4);
    let p = crate::theme::pal(&h.ctx);
    (painted(h.output()), rects(h.output()), p)
}

/// The viewer on `route`, to drive: settled, fonts and theme as the app
/// sets them.
pub(crate) fn harness(books: Books, route: Route) -> Harness<'static, App> {
    let mut h = Harness::builder()
        .with_size(egui::vec2(1400.0, 2400.0))
        .build_eframe(|cc| App::with_books(&cc.egui_ctx, books, false, route));
    h.run_steps(4);
    h
}

/// The painted text `text`, which must be there.
pub(crate) fn find<'a>(texts: &'a [Painted], text: &str) -> &'a Painted {
    texts.iter().find(|t| t.text == text).unwrap_or_else(|| {
        panic!(
            "{text:?} not painted: {:?}",
            texts.iter().map(|t| &t.text).collect::<Vec<_>>()
        )
    })
}

/// Whether `text` was painted.
pub(crate) fn has(texts: &[Painted], text: &str) -> bool {
    texts.iter().any(|t| t.text == text)
}

/// The text painted on the same line as `label`, to its right (a row's
/// amount).
pub(crate) fn beside<'a>(texts: &'a [Painted], label: &str) -> &'a Painted {
    let l = find(texts, label);
    texts
        .iter()
        .filter(|t| (t.mid_y() - l.mid_y()).abs() < 3.0 && t.pos.x > l.pos.x + l.width)
        .min_by(|a, b| a.pos.x.total_cmp(&b.pos.x))
        .unwrap_or_else(|| panic!("nothing beside {label:?}"))
}

/// The books of ledger-store's old-books fixture `name` (written by main
/// 188c90ea), as the viewer loads them.
pub(crate) fn old_books(name: &str) -> (TempDir, Books) {
    let from = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../ledger-store/tests/fixtures/old-books")
        .join(name);
    let n = DIRS.fetch_add(1, Ordering::Relaxed);
    let dir = TempDir(std::env::temp_dir().join(format!(
        "ledger-gui-unit-old-{name}-{}-{n}",
        std::process::id()
    )));
    let _ = std::fs::remove_dir_all(&dir.0);
    let data_dir = dir.0.join("books-data");
    std::fs::create_dir_all(data_dir.join("blobs")).unwrap();
    std::fs::copy(from.join("journal.jsonl"), data_dir.join("journal.jsonl")).unwrap();
    let books = data::load_once(&Source {
        data_dir,
        today: Some(today()),
    })
    .unwrap();
    (dir, books)
}
