//! Color schemes (schemes.rs): chosen from the menu in the sidebar's
//! corner, narrowed by the light/dark filter, kept with the viewer's
//! preferences. That every scheme paints the same words and figures in
//! the same places, each readable, is checked inside the crate
//! (schemes.rs, `painted`), where what is painted can be read.

use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use ledger_gui::nav::Route;
use ledger_gui::{App, data};
use std::collections::HashMap;

struct TestDir(std::path::PathBuf);

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn demo(tag: &str) -> (TestDir, data::Source) {
    let today = chrono::NaiveDate::from_ymd_opt(2026, 10, 12).unwrap();
    let dir = std::env::temp_dir().join(format!("ledger-gui-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    ledger_gui::demo::build(&dir, today).unwrap();
    let source = data::Source {
        data_dir: dir.join("books-data"),
        books_toml: Some(dir.join("wanderbooks.toml")),
        today: Some(today),
    };
    (TestDir(dir), source)
}

/// eframe's preferences, in memory.
#[derive(Default)]
struct Prefs(HashMap<String, String>);

impl eframe::Storage for Prefs {
    fn get_string(&self, key: &str) -> Option<String> {
        self.0.get(key).cloned()
    }
    fn set_string(&mut self, key: &str, value: String) {
        self.0.insert(key.to_string(), value);
    }
    fn remove_string(&mut self, key: &str) {
        self.0.remove(key);
    }
    fn flush(&mut self) {}
}

#[test]
fn a_scheme_is_chosen_from_the_menu_and_kept() {
    let (_dir, source) = demo("schemes-menu");
    let books = data::load_once(&source).unwrap();
    let mut h = Harness::builder()
        .with_size(egui::vec2(1440.0, 920.0))
        .build_eframe(|cc| App::with_books(&cc.egui_ctx, books, false, Route::Overview));
    h.run_steps(3);
    assert_eq!(h.state().scheme_key(), "Wanderbooks/Light");
    // The button names the scheme in force; it opens the menu.
    h.get_by_label_contains("Wanderbooks Light").click();
    h.run_steps(2);
    // Narrowed to dark schemes: a light variant isn't offered. (The
    // filter's "Dark" comes first; the viewer's own family, under the
    // pointer, may have opened its flyout with its own "Dark".)
    h.query_all_by_label("Dark").next().unwrap().click();
    h.run_steps(2);
    h.get_by_label_contains("Catppuccin").hover();
    h.run_steps(3);
    assert!(
        h.query_by_label("Latte").is_none(),
        "a light variant offered"
    );
    h.get_by_label("Mocha").click();
    h.run_steps(3);
    assert_eq!(h.state().scheme_key(), "Catppuccin/Mocha");
    // Worn: Mocha's own page color, and dark.
    let visuals = h.ctx.global_style().visuals.clone();
    assert!(visuals.dark_mode);
    assert_eq!(
        visuals.panel_fill,
        egui::Color32::from_rgb(0x18, 0x18, 0x25)
    );
    // Kept with the viewer's preferences, the filter too.
    let mut prefs = Prefs::default();
    eframe::App::save(h.state_mut(), &mut prefs);
    assert_eq!(prefs.0["scheme"], "Catppuccin/Mocha");
    assert_eq!(prefs.0["scheme_filter"], "Dark");
    // Opened again: only the family in force carries the check, and the
    // variant in force is selected under its name alone.
    let check = egui_phosphor::regular::CHECK;
    h.get_by_label_contains("Catppuccin Mocha").click();
    h.run_steps(2);
    // A family's row (not the scheme button, which names the scheme).
    let family = |h: &Harness<'_, App>, name: &str| -> String {
        h.query_all_by_label_contains(name)
            .map(|n| n.accesskit_node().label().unwrap_or_default().to_string())
            .find(|l| !l.contains(egui_phosphor::regular::PALETTE))
            .unwrap_or_else(|| panic!("no {name} row"))
    };
    let label = family(&h, "Catppuccin");
    assert!(label.contains(check), "{label:?}");
    let label = family(&h, "Gruvbox");
    assert!(!label.contains(check), "{label:?}");
    h.query_all_by_label_contains("Catppuccin")
        .find(|n| {
            !n.accesskit_node()
                .label()
                .unwrap_or_default()
                .contains(egui_phosphor::regular::PALETTE)
        })
        .unwrap()
        .hover();
    h.run_steps(3);
    let on = |name: &str| format!("{:?}", h.get_by_label(name).accesskit_node().toggled());
    assert_eq!(on("Mocha"), "Some(True)");
    assert_eq!(on("Macchiato"), "Some(False)");
}

/// The families in the menu, alphabetically (owner, 2026-10-19).
const FAMILIES: [&str; 21] = [
    "Ayu",
    "Catppuccin",
    "Cyberdream",
    "Dracula",
    "Everforest",
    "GitHub",
    "Gruvbox",
    "Kanagawa",
    "Material",
    "Melange",
    "Monokai Pro",
    "Night Owl",
    "Nightfox",
    "Nord",
    "One Dark",
    "Oxocarbon",
    "Rosé Pine",
    "Solarized",
    "Sonokai",
    "Tokyo Night",
    "Wanderbooks",
];

/// A family's row in the open menu (not the scheme button, which names
/// the scheme in force).
fn family_row<'a>(h: &'a Harness<'_, App>, name: &'a str) -> egui_kittest::Node<'a> {
    h.query_all_by_label_contains(name)
        .find(|n| {
            let label = n.accesskit_node().label().unwrap_or_default().to_string();
            !label.contains(egui_phosphor::regular::PALETTE)
                // The check (in force) and the flyout's arrow aside.
                && label.trim_matches(|c: char| !c.is_alphanumeric()) == name
        })
        .unwrap_or_else(|| panic!("no {name} row"))
}

/// Owner, 2026-10-19: Ayu was out of reach (the list scrolled, with no
/// sign of it), the families weren't alphabetical, and a click on a
/// variant's colors did nothing. Every family is on screen at once, in
/// order, in a short window or a tall one, and a click anywhere on a
/// variant's row chooses it.
#[test]
fn every_family_is_on_screen_in_order_and_a_whole_row_chooses() {
    let (_dir, source) = demo("schemes-reach");
    for (w, h_) in [(1440.0, 920.0), (1000.0, 480.0)] {
        let books = data::load_once(&source).unwrap();
        let mut h = Harness::builder()
            .with_size(egui::vec2(w, h_))
            .build_eframe(|cc| App::with_books(&cc.egui_ctx, books, false, Route::Overview));
        h.run_steps(3);
        h.get_by_label_contains("Wanderbooks Light").click();
        h.run_steps(3);
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(w, h_));
        let mut last: Option<egui::Rect> = None;
        for family in FAMILIES {
            let r = family_row(&h, family).rect();
            assert!(
                screen.contains_rect(r),
                "{w}x{h_}: {family} at {r:?} is off screen"
            );
            // In order: down a column, then on to the next one.
            if let Some(prev) = last {
                assert!(
                    r.top() > prev.top() || r.left() > prev.left(),
                    "{w}x{h_}: {family} comes before the family above it"
                );
            }
            last = Some(r);
        }
        // Ayu's flyout; a click on Mirage's colors, not its name.
        family_row(&h, "Ayu").hover();
        h.run_steps(3);
        let row = h.get_by_label("Mirage").rect();
        let on_colors = egui::pos2(row.left() + 20.0, row.center().y);
        h.hover_at(on_colors);
        h.run_steps(1);
        h.drag_at(on_colors);
        h.run_steps(1);
        h.drop_at(on_colors);
        h.run_steps(3);
        assert_eq!(h.state().scheme_key(), "Ayu/Mirage", "{w}x{h_}");
        assert!(row.width() >= 180.0, "the row is the button: {row:?}");
    }
}

/// Pictures of the menu and of a few schemes, for a person to look at
/// (`SHOT_DIR=… cargo test -p ledger-gui --test schemes -- --ignored`).
#[test]
#[ignore]
fn pictures() {
    let out = std::path::PathBuf::from(std::env::var("SHOT_DIR").expect("SHOT_DIR"));
    std::fs::create_dir_all(&out).unwrap();
    let (_dir, source) = demo("schemes-shots");
    let shot_at = |size: egui::Vec2, key: &str, menu: Option<&str>, name: &str| {
        let books = data::load_once(&source).unwrap();
        let mut h = Harness::builder()
            .with_size(size)
            .build_eframe(|cc| App::with_books(&cc.egui_ctx, books, false, Route::Overview));
        h.run_steps(2);
        let ctx = h.ctx.clone();
        h.state_mut().set_scheme(&ctx, key);
        h.run_steps(4);
        if let Some(family) = menu {
            let label = h.state().scheme_key().replace('/', " ");
            h.get_by_label_contains(label.split(' ').next().unwrap())
                .click();
            h.run_steps(3);
            h.get_by_label_contains(family).hover();
            h.run_steps(4);
        }
        h.render()
            .unwrap()
            .save(out.join(format!("{name}.png")))
            .unwrap();
    };
    let shot = |key: &str, menu: Option<&str>, name: &str| {
        shot_at(egui::vec2(1440.0, 920.0), key, menu, name)
    };
    shot("Wanderbooks/Light", Some("Ayu"), "menu");
    shot_at(
        egui::vec2(1000.0, 480.0),
        "Wanderbooks/Light",
        Some("Melange"),
        "menu-short",
    );
    for key in [
        "Catppuccin/Mocha",
        "Catppuccin/Latte",
        "Tokyo Night/Night",
        "Gruvbox/Dark",
        "Ayu/Light",
        "Kanagawa/Lotus",
        "Nord/Nord",
        "Solarized/Light",
    ] {
        shot(key, None, &key.replace(['/', ' '], "-"));
    }
}
