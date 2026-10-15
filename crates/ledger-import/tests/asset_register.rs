//! Spec tests for the fixed asset register PDF
//! (`ledger_import::asset_register`); the XLSX checks live in
//! spec_import.rs beside its workbook reader. Each test names the module
//! INVARIANT it checks.

use chrono::NaiveDate;
use ledger_core::{AssetRegister, AssetRegisterRow, TaxBasisRule};
use ledger_import::asset_register::{
    layout, missing_line, notes, rule_cited, rule_label, typeset_asset_register,
};
use ledger_import::report::ReportMeta;
use ledger_import::{PdfError, asset_register_pdf};
use rust_decimal::Decimal;
use typst::layout::{Frame, FrameItem};

fn d(y: i32, m: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, day).unwrap()
}

fn cents(c: i64) -> Decimal {
    Decimal::new(c, 2)
}

fn meta() -> ReportMeta {
    ReportMeta {
        company: "Wanderland Labs LLC".into(),
        basis: "Accrual".into(),
        prepared_on: d(2026, 10, 15),
        journal_seq: 314,
    }
}

fn row(id: u64, tag: &str, desc: &str, cost: i64, acc: i64) -> AssetRegisterRow {
    AssetRegisterRow {
        asset_id: id,
        tag: Some(tag.into()),
        description: desc.into(),
        serial: None,
        in_service: d(2026, 1, 15),
        cost: cents(cost),
        salvage: Decimal::ZERO,
        useful_life_months: 36,
        accumulated: cents(acc),
        book_value: cents(cost - acc),
        disposed: None,
        tax_rule: None,
        contributor_basis: None,
        tax_basis: None,
        built_in_gain: None,
    }
}

/// Bought: tax basis is cost.
fn bought(mut r: AssetRegisterRow) -> AssetRegisterRow {
    r.tax_rule = Some(TaxBasisRule::Cost);
    r.tax_basis = Some(r.cost);
    r
}

/// Contributed under `rule` with the member's basis `basis` (cents); tax
/// basis and built-in gain as ledger_core::Asset computes them.
fn contributed(mut r: AssetRegisterRow, rule: TaxBasisRule, basis: i64) -> AssetRegisterRow {
    let b = cents(basis);
    r.tax_rule = Some(rule);
    r.contributor_basis = Some(b);
    r.tax_basis = Some(match rule {
        TaxBasisRule::PersonalUseConversion => b.min(r.cost),
        _ => b,
    });
    r.built_in_gain = Some(r.cost - b);
    r
}

fn register(rows: Vec<AssetRegisterRow>) -> AssetRegister {
    let held = || rows.iter().filter(|r| r.disposed.is_none());
    let total_cost = held().map(|r| r.cost).sum();
    let total_accumulated = held().map(|r| r.accumulated).sum();
    let total_tax_basis = held().filter_map(|r| r.tax_basis).sum();
    let missing_tax_basis = held().filter(|r| r.tax_basis.is_none()).count();
    AssetRegister {
        total_tax_basis,
        missing_tax_basis,
        as_of: d(2026, 10, 15),
        total_book_value: total_cost - total_accumulated,
        ledger_cost: total_cost,
        ledger_accumulated: total_accumulated - cents(1234),
        total_cost,
        total_accumulated,
        rows,
    }
}

fn sample() -> AssetRegister {
    let mut disposed = row(3, "WL-0003", "Old monitor", 40000, 0);
    disposed.book_value = Decimal::ZERO;
    disposed.disposed = Some((d(2026, 9, 30), cents(4500)));
    let mut laptop = row(
        1,
        "WL-0001",
        "Framework Laptop 13 (Ryzen 7 7840U, 64 GB, 4 TB)",
        250000,
        52778,
    );
    laptop.serial = Some("FRANBMCP0A1234".into());
    laptop.salvage = cents(10000);
    register(vec![
        bought(laptop),
        // A member's business laptop: carryover basis, built-in gain.
        contributed(
            row(2, "WL-0002", "MacBook Air", 129900, 32475),
            TaxBasisRule::Carryover,
            90000,
        ),
        bought(disposed),
        // A member's personal camera: worth less than its basis, so tax
        // basis is the fair market value and there's a built-in loss.
        contributed(
            row(4, "WL-0004", "Sony A7 IV camera", 165000, 0),
            TaxBasisRule::PersonalUseConversion,
            191721,
        ),
        // No tax basis recorded yet.
        row(5, "WL-0005", "Standing desk", 80000, 0),
    ])
}

fn texts(frame: &Frame, out: &mut String) {
    for (_, item) in frame.items() {
        match item {
            FrameItem::Group(g) => texts(&g.frame, out),
            FrameItem::Text(t) => {
                out.push_str(&t.text);
                out.push(' ');
            }
            _ => {}
        }
    }
}

fn all_text(r: &AssetRegister, m: &ReportMeta) -> String {
    let doc = typeset_asset_register(r, m).expect("typesets");
    let mut out = String::new();
    for p in doc.pages() {
        texts(&p.frame, &mut out);
    }
    // Runs end at line breaks: collapse the seams to single spaces.
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// INVARIANT 6: the same register renders to the same bytes.
#[test]
fn pdf_is_deterministic() {
    let r = sample();
    let a = asset_register_pdf(&r, &meta()).unwrap();
    assert!(a.starts_with(b"%PDF"));
    assert_eq!(a, asset_register_pdf(&r, &meta()).unwrap());
}

/// INVARIANTS 1, 2, 3, 5: header, every asset, the status of each, the
/// register's totals, and the reconciliation with both differences.
#[test]
fn pdf_shows_assets_totals_and_reconciliation() {
    let t = all_text(&sample(), &meta());
    for s in [
        "Wanderland Labs LLC",
        "Fixed Asset Register",
        "As of October 15, 2026",
        "WL-0001",
        "FRANBMCP0A1234",
        "MacBook Air",
        "Active",
        "Disposed 2026-09-30, proceeds 45.00",
        "Total, assets held",
        "6,249.00", // cost held: 2,500.00 + 1,299.00 + 1,650.00 + 800.00
        "852.53",   // accumulated held
        "5,396.47", // book value held
        "Cost per register",
        "Cost per books",
        "Difference (unregistered cost)",
        "Accumulated depreciation per schedule",
        "Accumulated depreciation per books",
        "Difference (not yet posted)",
        "12.34",
        "Journal seq 314",
    ] {
        assert!(t.contains(s), "{s:?} missing from {t:?}");
    }
    // The disposed asset's cost (400.00) is shown but in no total.
    assert!(t.contains("400.00"));
    let l = layout(&sample());
    assert_eq!(l.totals, [cents(624900), cents(85253), cents(539647)]);
}

/// INVARIANTS 7-9: the tax table shows each rule in short words, the
/// contributor's basis, the tax basis, the built-in gain and loss, the
/// register's tax basis total, the missing line, and the notes with
/// their citations.
#[test]
fn pdf_shows_tax_basis() {
    let r = sample();
    let t = all_text(&r, &meta());
    for s in [
        "Federal tax basis",
        "TAX BASIS RULE",
        "CONTRIBUTOR'S BASIS",
        "BUILT-IN GAIN (LOSS)",
        "Carryover",
        "Personal-use conversion",
        "900.00",   // MacBook: contributor's basis = tax basis
        "399.00",   // its built-in gain
        "1,917.21", // camera: contributor's basis
        "1,650.00", // its tax basis: the fair market value
        "(267.21)", // its built-in loss
        "Not recorded",
        "Total tax basis, assets held",
        "5,050.00", // 2,500.00 + 900.00 + 1,650.00; the desk has none
        "Tax basis not recorded for 1 asset",
        "Notes",
        "Book value is the fair market value at contribution",
        "IRC §1012(a)",
        "IRC §723",
        "Treas. Reg. §1.168(i)-4(b)",
        "IRC §704(c)(1)(C)",
    ] {
        assert!(t.contains(s), "{s:?} missing from {t:?}");
    }
    let l = layout(&r);
    assert_eq!(l.tax_total, r.total_tax_basis);
    assert_eq!(l.tax_total, cents(505000));
    assert_eq!(
        l.missing.as_deref(),
        Some("Tax basis not recorded for 1 asset")
    );
    assert_eq!(l.notes, notes());
    // Each line carries its row's tax fields unchanged.
    for (line, row) in l.lines.iter().zip(&r.rows) {
        assert_eq!(line.tax_rule, row.tax_rule);
        assert_eq!(line.contributor_basis, row.contributor_basis);
        assert_eq!(line.tax_basis, row.tax_basis);
        assert_eq!(line.built_in_gain, row.built_in_gain);
    }
}

/// INVARIANT 8: no missing line when every asset held has a tax basis
/// (a disposed one without doesn't count), and the plural reads right.
#[test]
fn missing_line_only_when_missing() {
    let mut gone = row(1, "WL-0001", "Old monitor", 40000, 0);
    gone.book_value = Decimal::ZERO;
    gone.disposed = Some((d(2026, 9, 30), cents(4500)));
    let r = register(vec![bought(row(2, "WL-0002", "Desk", 50000, 0)), gone]);
    assert_eq!(r.missing_tax_basis, 0);
    assert_eq!(layout(&r).missing, None);
    assert!(!all_text(&r, &meta()).contains("Tax basis not recorded"));
    assert_eq!(missing_line(0), None);
    assert_eq!(
        missing_line(2).as_deref(),
        Some("Tax basis not recorded for 2 assets")
    );
}

/// INVARIANT 7: the rules in words, short and with their citations.
#[test]
fn rule_words() {
    use TaxBasisRule::*;
    assert_eq!(
        [Cost, Carryover, PersonalUseConversion].map(rule_label),
        ["Cost", "Carryover", "Personal-use conversion"]
    );
    assert_eq!(
        [Cost, Carryover, PersonalUseConversion].map(rule_cited),
        [
            "Cost",
            "Carryover (IRC §723)",
            "Personal-use conversion (Treas. Reg. §1.168(i)-4(b))"
        ]
    );
}

/// INVARIANT 4: an empty register renders a "No fixed assets" line, its
/// totals, and the reconciliation.
#[test]
fn empty_register_renders() {
    let r = register(vec![]);
    let pdf = asset_register_pdf(&r, &meta()).expect("empty renders");
    assert!(pdf.starts_with(b"%PDF"));
    let t = all_text(&r, &meta());
    for s in [
        "No fixed assets",
        "Total, assets held",
        "Difference (not yet posted)",
    ] {
        assert!(t.contains(s), "{s:?} missing from {t:?}");
    }
}

/// INVARIANT 6: markup in every text field prints literally and never runs.
#[test]
fn text_is_data() {
    let mut a = row(1, "#WL-*1*", "#panic() *bold* $x$ <l> @r [x] \\", 100, 0);
    a.serial = Some("_u_ // /*".into());
    let m = ReportMeta {
        company: "#co *Inc*".into(),
        ..meta()
    };
    let t = all_text(&register(vec![a]), &m);
    for s in [
        "#WL-*1*", "#panic()", "*bold*", "$x$", "<l>", "@r", "[x]", "_u_", "#co", "*Inc*",
    ] {
        assert!(t.contains(s), "{s:?} not literal in {t:?}");
    }
    for s in [
        "#panic()", "*", "_", "$", "[", "]", "\\", "#", "<a>", "@a", "//", "/*", "\"", "`",
    ] {
        let mut a = row(1, s, s, 100, 0);
        a.serial = Some(s.into());
        let m = ReportMeta {
            company: s.into(),
            ..meta()
        };
        let r: Result<Vec<u8>, PdfError> = asset_register_pdf(&register(vec![a]), &m);
        r.unwrap_or_else(|e| panic!("{s:?}: {e}"));
    }
}

/// A long register flows onto more landscape pages instead of being cut.
#[test]
fn long_register_paginates_landscape() {
    let rows = (1..=120)
        .map(|i| {
            row(
                i,
                &format!("WL-{i:04}"),
                &format!("Asset {i}"),
                100_000,
                1_000,
            )
        })
        .collect();
    let doc = typeset_asset_register(&register(rows), &meta()).unwrap();
    assert!(doc.pages().len() > 1);
    let size = doc.pages()[0].frame.size();
    assert!(size.x > size.y, "landscape");
}

/// The sample register for eyeballing: writes `$REGISTER_SAMPLE` (a path).
#[test]
#[ignore = "writes a sample PDF for a human to look at"]
fn write_sample() {
    let out = std::env::var("REGISTER_SAMPLE").expect("set REGISTER_SAMPLE to the output path");
    std::fs::write(out, asset_register_pdf(&sample(), &meta()).unwrap()).unwrap();
}
