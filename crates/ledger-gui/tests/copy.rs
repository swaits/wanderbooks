//! Click to copy: values leave the viewer exactly.
//!
//! ```text
//! GOAL: what people retype into forms (the EIN, an amount, an entry
//!       number) can be copied instead, and copies exactly.
//! SOURCE: DECISION (Stephen, 2026-10-14: click-to-copy on the company
//!         page, amounts, and identifiers).
//! INVARIANTS:
//!   1. An amount copies as its plain decimal: fmt::plain of money,
//!      accounting, or dollars text is the exact value, with "-" for a
//!      negative and no "$", commas, or parentheses.
//!   2. Clicking a company value or an entry's amount puts exactly that
//!      value on the clipboard.
//! ```

use egui::OutputCommand;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use ledger_gui::nav::Route;
use ledger_gui::{App, Source, data, fmt};
use proptest::prelude::*;
use rust_decimal::Decimal;

proptest! {
    /// INVARIANT 1: every display style copies back to the same value.
    #[test]
    fn an_amount_copies_as_its_exact_value(
        mantissa in -10i128.pow(16)..10i128.pow(16),
        scale in 0u32..8,
    ) {
        let d = Decimal::from_i128_with_scale(mantissa, scale);
        for shown in [fmt::money(d), fmt::accounting(d), fmt::dollars(d)] {
            let plain = fmt::plain(&shown).expect("an amount");
            prop_assert!(!plain.contains(['$', ',', '(', ')', '\u{2212}']), "{}", plain);
            prop_assert_eq!(plain.parse::<Decimal>().unwrap(), d, "{} copied as {}", shown, plain);
        }
    }
}

fn copied(h: &Harness<'_, App>) -> Vec<String> {
    h.output()
        .platform_output
        .commands
        .iter()
        .filter_map(|c| match c {
            OutputCommand::CopyText(t) => Some(t.clone()),
            _ => None,
        })
        .collect()
}

/// INVARIANT 2, on the demo books.
#[test]
fn clicking_a_value_copies_it() {
    let dir = std::env::temp_dir().join(format!("ledger-gui-copy-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let today = chrono::NaiveDate::from_ymd_opt(2026, 10, 12).unwrap();
    ledger_gui::demo::build(&dir, today).unwrap();
    let source = Source {
        data_dir: dir.join("books-data"),
        books_toml: Some(dir.join("wanderbooks.toml")),
        today: Some(today),
    };
    let open = |route: Route| {
        let books = data::load_once(&source).unwrap();
        let mut h = Harness::builder()
            .with_size(egui::vec2(1280.0, 900.0))
            .build_eframe(|cc| App::with_books(&cc.egui_ctx, books, false, route));
        h.run_steps(3);
        h
    };

    // The company's EIN, as written in wanderbooks.toml.
    let ein = data::load_once(&source)
        .unwrap()
        .settings
        .company
        .profile
        .iter()
        .find(|(k, _)| k == "ein")
        .map(|(_, v)| v.clone())
        .expect("the demo has an EIN");
    let mut h = open(Route::Company);
    h.get_by_label(&ein).click();
    h.step();
    assert_eq!(copied(&h), [ein]);

    // An entry's amount, as a plain decimal.
    let books = data::load_once(&source).unwrap();
    let e = books
        .entries
        .iter()
        .find(|e| e.amount.is_some_and(|a| !a.is_zero()))
        .unwrap();
    let (id, amount) = (e.id, e.amount.unwrap());
    drop(books);
    let mut h = open(Route::Entry(id));
    let shown = fmt::dollars(amount);
    h.get_by_label(&shown).click();
    h.step();
    assert_eq!(copied(&h), [fmt::plain(&shown).unwrap()]);
    assert_eq!(copied(&h)[0].parse::<Decimal>().unwrap(), amount);

    // The entry number.
    let mut h = open(Route::Entry(id));
    h.get_by_label(&format!("Entry #{id}")).click();
    h.step();
    assert_eq!(copied(&h), [format!("#{id}")]);
    let _ = std::fs::remove_dir_all(&dir);
}
