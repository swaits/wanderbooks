//! K-U9: one display rounding, and no float in a money path.
//!
//! The behavioural half uses the public formatting API. The source
//! half is a scoped grep: the display files that shorten or ratio
//! money must not name `round_dp` (they go through
//! `ledger_import::round_display` or `kernel::percent_of`), and a
//! float conversion of a decimal may appear only at the xlsx cell
//! boundary, where the cell itself is a binary double and the value
//! is written as a number only when it is exactly representable.

use rust_decimal::Decimal;
use std::path::{Path, PathBuf};
use std::str::FromStr;

fn dec(s: &str) -> Decimal {
    Decimal::from_str(s).unwrap()
}

fn crate_src(krate: &str, file: &str) -> String {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    std::fs::read_to_string(root.join(krate).join("src").join(file))
        .unwrap_or_else(|e| panic!("{krate}/src/{file}: {e}"))
}

fn all_rs(dir: &Path, out: &mut Vec<PathBuf>) {
    for e in std::fs::read_dir(dir).unwrap() {
        let p = e.unwrap().path();
        if p.is_dir() {
            all_rs(&p, out);
        } else if p.extension().is_some_and(|x| x == "rs") {
            out.push(p);
        }
    }
}

/// Display rounding: half away from zero, never a negative zero.
#[test]
fn display_rounding_is_half_away_and_never_negative_zero() {
    let r = ledger_import::round_display;
    assert_eq!(r(dec("1.005"), 2), dec("1.01"));
    assert_eq!(r(dec("-1.005"), 2), dec("-1.01"));
    assert_eq!(r(dec("2.5"), 0), dec("3"));
    assert_eq!(r(dec("-0.004"), 2).to_string(), "0.00");
    assert_eq!(ledger_import::format_amount(dec("-0.004")), "0.00");
    assert_eq!(ledger_import::format_amount(dec("-1.005")), "(1.01)");
    // The viewer re-exports the same function.
    assert_eq!(ledger_gui::fmt::round_display(dec("1.005"), 2), dec("1.01"));
}

/// A percentage of a zero or negative base is shown as nothing.
#[test]
fn a_percent_of_a_zero_or_negative_base_is_none() {
    assert_eq!(
        ledger_gui::fmt::pct_change(dec("200"), dec("250")),
        Some(dec("25.0"))
    );
    assert_eq!(ledger_gui::fmt::pct_change(dec("0"), dec("5")), None);
    assert_eq!(ledger_gui::fmt::pct_change(dec("-100"), dec("-50")), None);
}

/// No `round_dp` left in the display paths, and no float conversion
/// of a decimal outside the xlsx cell boundary.
#[test]
fn no_round_dp_or_float_conversion_in_money_display_paths() {
    for (krate, file) in [
        ("ledger-gui", "fmt.rs"),
        ("ledger-gui", "books.rs"),
        ("ledger-gui", "demo.rs"),
        ("ledger-gui", "pages/profit.rs"),
        ("ledger-gui", "pages/reports.rs"),
        ("ledger-gui", "pages/overview.rs"),
        ("ledger-gui", "pages/cash.rs"),
        ("ledger-gui", "pages/capital.rs"),
        ("ledger-import", "report.rs"),
        ("ledger-import", "pdf.rs"),
        ("ledger-import", "xlsx.rs"),
        ("ledger-import", "format.rs"),
        ("ledger-package", "text.rs"),
    ] {
        assert!(
            !crate_src(krate, file).contains("round_dp"),
            "{krate}/src/{file} must use round_display / kernel rounding"
        );
    }
    // fmt's percent and compact paths (everything above `bytes`, which
    // sizes a byte count, not money) name no float at all.
    let fmt_src = crate_src("ledger-gui", "fmt.rs");
    let (money_half, _) = fmt_src
        .split_once("pub fn bytes")
        .expect("fmt::bytes exists");
    assert!(
        !money_half.contains("f64"),
        "fmt money paths must be Decimal"
    );
    assert!(
        !money_half.contains("f32"),
        "fmt money paths must be Decimal"
    );

    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    for krate in [
        "ledger-gui",
        "ledger-import",
        "ledger-package",
        "ledger-mcp",
    ] {
        let mut files = Vec::new();
        all_rs(&root.join(krate).join("src"), &mut files);
        for f in files {
            let text = std::fs::read_to_string(&f).unwrap();
            if text.contains("to_f64") {
                assert!(
                    f.ends_with("xlsx.rs"),
                    "to_f64 only at the xlsx cell boundary: {f:?}"
                );
            }
        }
    }
}
