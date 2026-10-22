//! The sidebar's links fit the window (owner, 2026-10-22: at 1479x519
//! half the links hid under the status box with no sign that they
//! scroll). As the window gets shorter the sidebar gives way in three
//! steps, each only when the one before isn't enough: the status card
//! folds to its summary line, then the rows tighten, then the links
//! scroll, with a fade and a chevron at the clipped edge.
//!
//! ```text
//! GOAL: every link reachable and never under the footer, at any height
//!       and scale, sidebar and rail; a window where everything fits
//!       looks exactly as it did.
//! SOURCE: app.rs (Fit, fit, sidebar, rail, live_status).
//! ```
//!
//! `SHOT_DIR` keeps pictures for eyes (`sidenav_screenshots`).
//! `SIDENAV_BLESS=1` rewrites the goldens of the 1440x920 sidebar
//! (`tests/goldens/`): only from a commit whose sidebar is the one meant.

use chrono::NaiveDate;
use egui::{Rect, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use ledger_core::{AccountType, Currency, Money, Side};
use ledger_gui::books::Books;
use ledger_gui::nav::{Route, Section};
use ledger_gui::owner::{Agent, Owner, RestorePlan};
use ledger_gui::{App, Source, data};
use ledger_store::{DiskCheck, Store};
use rust_decimal::Decimal;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

/// An owner that never gets asked anything here: it only makes the
/// viewer show its Books menu, as the live one does.
struct Quiet;

impl Owner for Quiet {
    fn close_books(&self) -> Result<String, String> {
        Err("not here".into())
    }
    fn new_books(&self, _: &str, _: &str) -> Result<String, String> {
        Err("not here".into())
    }
    fn back_up_now(&self, _: &str) -> Result<Vec<String>, String> {
        Err("not here".into())
    }
    fn plan_restore(&self, _: &str, _: Option<String>, _: bool) -> Result<RestorePlan, String> {
        Err("not here".into())
    }
    fn restore(&self, _: &RestorePlan) -> Result<Vec<String>, String> {
        Err("not here".into())
    }
    fn connect(&self, _: Agent, _: &str) -> Result<Vec<String>, String> {
        Err("not here".into())
    }
    fn upgrade_format(&self, _: &str, _: &str) -> Result<Vec<String>, String> {
        Err("not here".into())
    }
}

fn owner() {
    static ONCE: OnceLock<()> = OnceLock::new();
    ONCE.get_or_init(|| ledger_gui::owner::set_owner(Arc::new(Quiet)));
}

fn today() -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 10, 22).unwrap()
}

/// What the sidebar's footer can show: the live viewer's (a Books menu,
/// Live, every check passing) and each way it differs.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Footer {
    /// The owner's: Books menu, Live, every check passing.
    Owners,
    /// Not following a journal: "Snapshot".
    Snapshot,
    Behind,
    Ahead,
    /// A copy of the books: no server database.
    Missing,
    Differs,
    Unreadable,
    /// Stored files damaged.
    Damaged,
    /// Books outside a data root: no Books menu.
    NoMenu,
}

const FOOTERS: [Footer; 9] = [
    Footer::Owners,
    Footer::Snapshot,
    Footer::Behind,
    Footer::Ahead,
    Footer::Missing,
    Footer::Differs,
    Footer::Unreadable,
    Footer::Damaged,
    Footer::NoMenu,
];

/// Books with a footer like the live one's: a data root's (its Books
/// menu), about 560 events and 110 stored receipts. Removed when dropped.
struct Fixture {
    _dir: tempfile::TempDir,
    root: PathBuf,
    plain: PathBuf,
}

fn usd(c: i64) -> Money {
    Money::new(Decimal::new(c, 2), Currency::USD)
}

fn fill(book: &Path) {
    let mut s = Store::create(book, Currency::USD).unwrap();
    let mut add = |code: &str, ty| s.add_account(code, code, ty, None, false).unwrap();
    let cash = add("1000", AccountType::Asset);
    let capital = add("3000", AccountType::Equity);
    let rent = add("6000", AccountType::Expense);
    for i in 0..110i64 {
        let (dr, cr) = if i == 0 {
            (cash, capital)
        } else {
            (rent, cash)
        };
        let e = s
            .create_entry(NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(), "test")
            .unwrap();
        s.add_line(e, dr, Side::Debit, usd(100 + i)).unwrap();
        s.add_line(e, cr, Side::Credit, usd(100 + i)).unwrap();
        s.attach_receipt(
            Some(e),
            "r.pdf",
            "application/pdf",
            format!("%PDF {i}").as_bytes(),
            None,
        )
        .unwrap();
        s.post_entry(e).unwrap();
    }
}

impl Fixture {
    fn new() -> Fixture {
        owner();
        let dir = tempfile::Builder::new()
            .prefix("wb-gui-sidenav-")
            .tempdir()
            .unwrap();
        std::fs::write(dir.path().join(ledger_store::root::MARKER_FILE), b"").unwrap();
        let root = dir.path().join("books").join("wanderland");
        std::fs::create_dir_all(&root).unwrap();
        fill(&root);
        // The same books outside the root: no Books menu.
        let plain = tempfile::Builder::new()
            .prefix("wb-gui-sidenav-plain-")
            .tempdir()
            .unwrap()
            .keep();
        let plain_book = plain.join("books-data");
        copy_dir(&root, &plain_book);
        Fixture {
            _dir: dir,
            root,
            plain: plain_book,
        }
    }

    fn books(&self, footer: Footer) -> Books {
        let data_dir = if footer == Footer::NoMenu {
            self.plain.clone()
        } else {
            self.root.clone()
        };
        let mut b = data::load_once(&Source {
            data_dir,
            today: Some(today()),
        })
        .unwrap();
        match footer {
            Footer::Behind => {
                b.integrity.disk = DiskCheck::Behind {
                    applied: 550,
                    journal: 559,
                }
            }
            Footer::Ahead => {
                b.integrity.disk = DiskCheck::Ahead {
                    applied: 561,
                    journal: 559,
                }
            }
            Footer::Missing => b.integrity.disk = DiskCheck::Missing,
            Footer::Differs => {
                b.integrity.disk = DiskCheck::Differs("the entries table differs".into())
            }
            Footer::Unreadable => {
                b.integrity.disk = DiskCheck::Unreadable("database disk image is malformed".into())
            }
            Footer::Damaged => {
                b.integrity.file_failures = vec![(3, "missing".into()), (9, "hash differs".into())]
            }
            _ => {}
        }
        b
    }

    /// The viewer on `route`, `w`x`h` points at `scale`.
    fn harness(
        &self,
        footer: Footer,
        route: Route,
        w: f32,
        h: f32,
        scale: f32,
    ) -> Harness<'static, App> {
        let b = self.books(footer);
        let mut h = Harness::builder()
            .with_size(vec2(w, h))
            .with_pixels_per_point(scale)
            .build_eframe(move |cc| {
                let mut a = App::with_books(&cc.egui_ctx, b, true, route);
                a.set_scheme(&cc.egui_ctx, "Everforest/Dark");
                if footer != Footer::Snapshot {
                    a.show_live_label();
                }
                a
            });
        h.run_steps(6);
        h
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        if let Some(p) = self.plain.parent() {
            let _ = std::fs::remove_dir_all(p);
        }
    }
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for e in std::fs::read_dir(from).unwrap() {
        let p = e.unwrap().path();
        let dest = to.join(p.file_name().unwrap());
        if p.is_dir() {
            copy_dir(&p, &dest);
        } else {
            std::fs::copy(&p, &dest).unwrap();
        }
    }
}

/// Sidebar or rail: which one the window's width shows.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Pane {
    Sidebar,
    Rail,
}

impl Pane {
    /// Its width.
    fn panel(self) -> f32 {
        match self {
            Pane::Sidebar => 224.0,
            Pane::Rail => 60.0,
        }
    }
    /// A window width (points) that shows it.
    fn width(self) -> f32 {
        match self {
            Pane::Sidebar => 1440.0,
            Pane::Rail => 900.0,
        }
    }
    fn key(self) -> &'static str {
        match self {
            Pane::Sidebar => "sidebar",
            Pane::Rail => "rail",
        }
    }
}

/// The link of section `s` (the leftmost button of its name).
fn link(h: &Harness<'_, App>, s: Section) -> Rect {
    h.get_all_by_role_and_label(egui::accesskit::Role::Button, s.title())
        .map(|n| n.rect())
        .min_by(|a, b| a.left().total_cmp(&b.left()))
        .unwrap_or_else(|| panic!("no {} link", s.title()))
}

/// What the sidebar says it drew: the links' viewport, the footer's
/// rect, and whether the links scroll.
fn drawn(h: &Harness<'_, App>, side: Pane) -> (Rect, Rect, bool) {
    let k = side.key();
    h.ctx.data(|d| {
        (
            d.get_temp::<Rect>(egui::Id::new(format!("{k}_links_view")))
                .expect("the links' viewport"),
            d.get_temp::<Rect>(egui::Id::new(format!("{k}_footer_rect")))
                .expect("the footer's rect"),
            d.get_temp::<bool>(egui::Id::new(format!("{k}_links_scroll")))
                .expect("whether the links scroll"),
        )
    })
}

const EPS: f32 = 0.01;

/// `r` shows top to bottom inside `view`. Links are laid on whole
/// pixels and the viewport needn't be, so at a fractional scale they may
/// differ by under a pixel (half a point at most here); across, a link may
/// overrun the viewport likewise.
fn whole_in(view: Rect, r: Rect) -> bool {
    r.top() >= view.top() - 0.5 && r.bottom() <= view.bottom() + 0.5
}

/// No link shows over or under the footer: the part of each link inside
/// the links' viewport ends above the footer and above every footer
/// widget the tree names; without scrolling every link shows whole.
fn assert_clear(h: &Harness<'_, App>, side: Pane, tag: &str) {
    let (view, footer, scrolls) = drawn(h, side);
    assert!(
        view.bottom() <= footer.top() + EPS,
        "{tag}: the links' viewport {view:?} reaches the footer {footer:?}"
    );
    // The footer's own widgets, found in the tree, not by the hook.
    let mut tops = vec![footer.top()];
    for label in ["Export", "event "] {
        for n in h.query_all_by_label_contains(label) {
            if n.rect().left() < side.panel() && n.rect().top() > view.top() {
                tops.push(n.rect().top());
            }
        }
    }
    let footer_top = tops.into_iter().fold(f32::INFINITY, f32::min);
    let mut whole = 0;
    for s in Section::ALL {
        let r = link(h, s);
        let seen = r.intersect(view);
        if seen.is_positive() {
            assert!(
                seen.bottom() <= footer_top + EPS,
                "{tag}: {} shows to {} under the footer at {footer_top}",
                s.title(),
                seen.bottom()
            );
        }
        if whole_in(view, r) {
            whole += 1;
        } else {
            assert!(
                scrolls,
                "{tag}: {} at {r:?} cut off with no scrolling in {view:?}",
                s.title()
            );
        }
    }
    assert!(
        whole >= 1,
        "{tag}: no link shows whole in {view:?} (footer {footer:?}): {:?}",
        Section::ALL.map(|s| link(h, s))
    );
}

/// Every height from 300 to 920 pixels in steps of 20, at scales 1 to 2,
/// sidebar and rail, with each footer the live viewer can show.
fn grid(scale: f32) {
    let f = Fixture::new();
    for footer in FOOTERS {
        for side in [Pane::Sidebar, Pane::Rail] {
            let mut h = f.harness(footer, Route::Projects, side.width(), 920.0, scale);
            for px in (300..=920).step_by(20) {
                let pt = px as f32 / scale;
                h.set_size(vec2(side.width(), pt));
                h.run_steps(4);
                let tag = format!("{footer:?} {side:?} {px}px @{scale}");
                assert_clear(&h, side, &tag);
                // The owner's complaint: in a window as tall as theirs
                // (519) or taller, no link is out of sight, sidebar or rail.
                if scale == 1.0 && px >= 519 {
                    assert!(!drawn(&h, side).2, "{tag}: the links scroll");
                }
            }
        }
    }
}

#[test]
fn no_link_is_ever_under_the_footer_at_scale_1() {
    grid(1.0);
}

#[test]
fn no_link_is_ever_under_the_footer_at_scale_1_25() {
    grid(1.25);
}

#[test]
fn no_link_is_ever_under_the_footer_at_scale_1_5() {
    grid(1.5);
}

#[test]
fn no_link_is_ever_under_the_footer_at_scale_2() {
    grid(2.0);
}

/// The owner's window: every link shows whole, nothing scrolls, and the
/// status card is its summary line, the checks in its tooltip.
#[test]
fn at_1479x519_every_link_shows_unscrolled() {
    let f = Fixture::new();
    for footer in FOOTERS {
        let mut h = f.harness(footer, Route::Projects, 1479.0, 519.0, 1.0);
        let (view, _, scrolls) = drawn(&h, Pane::Sidebar);
        assert!(!scrolls, "{footer:?}: the links scroll");
        for s in Section::ALL {
            let r = link(&h, s);
            assert!(
                whole_in(view, r) && r.bottom() <= 519.0,
                "{footer:?}: {} at {r:?} isn't whole in {view:?}",
                s.title()
            );
            assert!(r.height() >= 24.0, "{footer:?}: {} too tight", s.title());
        }
        assert_clear(&h, Pane::Sidebar, &format!("{footer:?}"));
        // Nothing moves when the last link is asked into view.
        let first = link(&h, Section::Overview);
        h.get_all_by_role_and_label(egui::accesskit::Role::Button, "Journal")
            .min_by(|a, b| a.rect().left().total_cmp(&b.rect().left()))
            .unwrap()
            .scroll_to_me();
        h.run_steps(4);
        assert_eq!(
            link(&h, Section::Overview),
            first,
            "{footer:?}: it scrolled"
        );
    }
}

/// Folded, the card is one line that fits the card whole (never cut to
/// "Datab…"), naming the check that failed; hovered, it shows the three
/// checks.
#[test]
fn the_folded_card_shows_its_checks_on_hover() {
    let f = Fixture::new();
    for (footer, verdict, checks) in [
        (
            Footer::Owners,
            "verified",
            [
                "Journal chain verified",
                "Database matches journal",
                "110 files verified",
            ],
        ),
        (
            Footer::Snapshot,
            // Its line is "Snapshot · event 555 · (seal)": no room for
            // "verified", which the seal says.
            "Snapshot",
            [
                "Journal chain verified",
                "Database matches journal",
                "110 files verified",
            ],
        ),
        (
            Footer::Differs,
            "Database differs",
            [
                "Journal chain verified",
                "Database differs",
                "110 files verified",
            ],
        ),
        (
            Footer::Unreadable,
            "Database unreadable",
            [
                "Journal chain verified",
                "Database unreadable",
                "110 files verified",
            ],
        ),
        (
            Footer::Damaged,
            "2 files damaged",
            [
                "Journal chain verified",
                "Database matches journal",
                "2 files damaged",
            ],
        ),
    ] {
        for scale in [1.0, 1.25] {
            let tag = format!("{footer:?} @{scale}");
            let mut h = f.harness(
                footer,
                Route::Projects,
                1479.0 / scale,
                519.0 / scale,
                scale,
            );
            assert!(
                h.query_by_label_contains("files verified").is_none()
                    && h.query_by_label_contains("Journal chain verified")
                        .is_none(),
                "{tag}: the card is folded"
            );
            let (_, footer_rect, _) = drawn(&h, Pane::Sidebar);
            let line = h.get_by_label_contains(verdict);
            let card = line.rect();
            // Every piece of the line lies inside the card's margins: a
            // piece cut off or running out would reach past them.
            for piece in ["Live", "Snapshot", "event 5", verdict] {
                for n in h.query_all_by_label_contains(piece) {
                    let r = n.rect();
                    if r.left() < Pane::Sidebar.panel() && r.top() >= footer_rect.top() {
                        assert!(
                            r.right() <= footer_rect.right() - 10.0 + 0.5,
                            "{tag}: {piece:?} at {r:?} runs out of the card {footer_rect:?}"
                        );
                    }
                }
            }
            h.hover_at(card.center());
            h.run_steps(30);
            for c in checks {
                assert!(
                    h.query_all_by_label_contains(c).count() >= 1,
                    "{tag}: hovered, the card shows {c}"
                );
            }
        }
    }
}

/// The owner's height in the rail too: every link shows, unscrolled.
#[test]
fn the_rail_at_519_shows_every_link_unscrolled() {
    let f = Fixture::new();
    for footer in FOOTERS {
        for route in [Route::Projects, Route::Overview, Route::Journal] {
            let h = f.harness(footer, route.clone(), 900.0, 519.0, 1.0);
            let tag = format!("{footer:?} {route:?}");
            let (view, _, scrolls) = drawn(&h, Pane::Rail);
            assert!(!scrolls, "{tag}: the rail's links scroll");
            for s in Section::ALL {
                let r = link(&h, s);
                assert!(
                    whole_in(view, r) && r.bottom() <= 519.0,
                    "{tag}: {} at {r:?} isn't whole in {view:?}",
                    s.title()
                );
            }
            assert_clear(&h, Pane::Rail, &tag);
        }
    }
}

/// Where everything fits, the card shows its checks, unfolded.
#[test]
fn a_tall_window_keeps_the_whole_card() {
    let f = Fixture::new();
    let h = f.harness(Footer::Owners, Route::Projects, 1440.0, 920.0, 1.0);
    for c in [
        "Journal chain verified",
        "Database matches journal",
        "110 files verified",
    ] {
        assert!(h.query_by_label_contains(c).is_some(), "{c}");
    }
    let (_, _, scrolls) = drawn(&h, Pane::Sidebar);
    assert!(!scrolls);
}

/// Too short even for tight rows: the links scroll, the current page's
/// link is scrolled into view, and a chevron says more links are there.
#[test]
fn scrolling_links_show_the_current_one_and_a_cue() {
    let f = Fixture::new();
    for side in [Pane::Sidebar, Pane::Rail] {
        let h = f.harness(Footer::Owners, Route::Journal, side.width(), 300.0, 1.0);
        let (view, _, scrolls) = drawn(&h, side);
        assert!(scrolls, "{side:?}: 300 points can't hold every link");
        let journal = link(&h, Section::Journal);
        assert!(
            whole_in(view, journal),
            "{side:?}: Journal at {journal:?} not in {view:?}"
        );
        assert!(
            h.query_by_label("More links above").is_some(),
            "{side:?}: the cue above"
        );
        let mut h = f.harness(Footer::Owners, Route::Overview, side.width(), 300.0, 1.0);
        assert!(
            h.query_by_label("More links below").is_some(),
            "{side:?}: the cue below"
        );
        assert!(h.query_by_label("More links above").is_none(), "{side:?}");
        // The chevron scrolls the links.
        let before = link(&h, Section::Overview);
        h.get_by_label("More links below").click();
        h.run_steps(6);
        assert!(
            link(&h, Section::Overview).top() < before.top(),
            "{side:?}: the chevron scrolled"
        );
    }
}

/// Going to a page scrolls its link into view (the palette, a crumb).
#[test]
fn going_somewhere_scrolls_its_link_into_view() {
    let f = Fixture::new();
    let mut h = f.harness(Footer::Owners, Route::Overview, 1440.0, 300.0, 1.0);
    for (page, s) in [
        ("journal", Section::Journal),
        ("overview", Section::Overview),
        ("documents", Section::Documents),
    ] {
        h.state_mut()
            .show(ledger_store::ops::ViewerTarget::Page { page: page.into() });
        h.run_steps(6);
        let (view, _, _) = drawn(&h, Pane::Sidebar);
        let r = link(&h, s);
        assert!(whole_in(view, r), "{} at {r:?} in {view:?}", s.title());
    }
}

/// 1440x920, where everything always fit: pixel for pixel the sidebar
/// main drew (2d7b12a), and the same shapes.
#[test]
fn a_tall_sidebar_is_unchanged() {
    let f = Fixture::new();
    let mut h = f.harness(Footer::Owners, Route::Overview, 1440.0, 920.0, 1.0);
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/goldens");
    let shapes = sidebar_shapes(&h);
    let bless = std::env::var("SIDENAV_BLESS").is_ok();
    let shapes_file = dir.join("sidenav-1440x920.shapes");
    if bless {
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(&shapes_file, format!("{shapes:016x}\n")).unwrap();
    } else {
        let want = std::fs::read_to_string(&shapes_file).unwrap();
        assert_eq!(
            format!("{shapes:016x}"),
            want.trim(),
            "the sidebar's shapes at 1440x920 changed"
        );
    }
    // Pixels too, only on the machine that blessed them (SIDENAV_PIXELS=1):
    // another renderer rasterizes the same shapes differently (CI's
    // software renderer differs in antialiasing), so the shapes above are
    // the check everywhere and the pixels a local one.
    if !bless && std::env::var("SIDENAV_PIXELS").is_err() {
        return;
    }
    let png = dir.join("sidenav-1440x920.png");
    match h.render() {
        Ok(img) => {
            let side = image::imageops::crop_imm(&img, 0, 0, 224, img.height()).to_image();
            if bless {
                side.save(&png).unwrap();
            } else {
                let want = image::open(&png).unwrap().to_rgba8();
                assert_eq!(side.dimensions(), want.dimensions());
                let diff = side
                    .pixels()
                    .zip(want.pixels())
                    .filter(|(a, b)| a != b)
                    .count();
                assert_eq!(diff, 0, "{diff} sidebar pixels differ from main's");
            }
        }
        Err(e) => eprintln!("no renderer ({e}): pixels not compared, shapes were"),
    }
}

/// A hash of every shape painted in the sidebar's column (FNV-1a over
/// their debug form): the same shapes paint the same pixels.
fn sidebar_shapes(h: &Harness<'_, App>) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for s in &h.output().shapes {
        if s.clip_rect.max.x <= 224.5 {
            for b in format!("{:?}", s.shape).bytes() {
                hash ^= b as u64;
                hash = hash.wrapping_mul(0x100_0000_01b3);
            }
        }
    }
    hash
}

/// Pictures for eyes: set `SHOT_DIR`.
#[test]
fn sidenav_screenshots() {
    let Ok(out) = std::env::var("SHOT_DIR") else {
        return;
    };
    let out = PathBuf::from(out);
    std::fs::create_dir_all(&out).unwrap();
    let f = Fixture::new();
    let shots: [(&str, Route, f32, f32, f32); 11] = [
        ("sidebar-1479x519", Route::Projects, 1479.0, 519.0, 1.0),
        (
            "sidebar-1479x519-overview",
            Route::Overview,
            1479.0,
            519.0,
            1.0,
        ),
        (
            "sidebar-1479x519-journal",
            Route::Journal,
            1479.0,
            519.0,
            1.0,
        ),
        ("sidebar-1479x420", Route::Projects, 1479.0, 420.0, 1.0),
        ("sidebar-1479x340", Route::Projects, 1479.0, 340.0, 1.0),
        (
            "sidebar-1479x340-journal",
            Route::Journal,
            1479.0,
            340.0,
            1.0,
        ),
        ("sidebar-1440x920", Route::Overview, 1440.0, 920.0, 1.0),
        // 986x346 points: narrow enough for the rail.
        ("rail-1479x519-scale1.5", Route::Projects, 986.0, 346.0, 1.5),
        ("rail-900x519", Route::Projects, 900.0, 519.0, 1.0),
        ("rail-900x340", Route::Projects, 900.0, 340.0, 1.0),
        ("rail-900x920", Route::Projects, 900.0, 920.0, 1.0),
    ];
    for (name, route, w, hgt, scale) in shots {
        let mut h = f.harness(Footer::Owners, route, w, hgt, scale);
        if let Ok(img) = h.render() {
            img.save(out.join(format!("{name}.png"))).unwrap();
        }
    }
    // The folded card in each footer, at the owner's size.
    for footer in FOOTERS {
        let mut h = f.harness(footer, Route::Projects, 1479.0, 519.0, 1.0);
        if let Ok(img) = h.render() {
            img.save(out.join(format!("folded-{footer:?}.png")))
                .unwrap();
        }
    }
    // The folded card's tooltip.
    let mut h = f.harness(Footer::Owners, Route::Projects, 1479.0, 519.0, 1.0);
    if let Some(n) = h.query_by_label_contains("event 5") {
        let c = n.rect().center();
        h.hover_at(c);
        h.run_steps(30);
        if let Ok(img) = h.render() {
            img.save(out.join("sidebar-1479x519-hover.png")).unwrap();
        }
    }
}
