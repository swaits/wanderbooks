//! Spec tests for the fixed asset register PDF
//! (`ledger_import::asset_register`); the XLSX checks live in
//! spec_import.rs beside its workbook reader. Each test names the module
//! INVARIANT it checks.

use chrono::NaiveDate;
use ledger_core::{AssetRegister, AssetRegisterRow, BasisRule, Sec704cMethod};
use ledger_import::asset_register::{
    FMV_NOT_RECORDED_MARK, layout, method_cited, missing_line, notes, rule_cited, rule_label,
    rule_mark, template_data, typeset_asset_register,
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
        company: "Fernhill Labs LLC".into(),
        basis: "Accrual".into(),
        prepared_on: d(2026, 10, 15),
        journal_seq: 314,
    }
}

/// A row: `scheduled` the schedule's accumulated depreciation, `posted`
/// what the books carry; book value is cost less posted.
fn row(id: u64, tag: &str, desc: &str, cost: i64, scheduled: i64, posted: i64) -> AssetRegisterRow {
    AssetRegisterRow {
        asset_id: id,
        tag: Some(tag.into()),
        description: desc.into(),
        serial: None,
        acquired_date: d(2026, 1, 15),
        in_service_date: d(2026, 1, 15),
        cost: cents(cost),
        salvage: Decimal::ZERO,
        useful_life_months: 36,
        accumulated_scheduled: cents(scheduled),
        accumulated_posted: cents(posted),
        book_value: cents(cost - posted),
        disposed: None,
        basis_rule: None,
        contributor_adjusted_basis: None,
        fmv_at_contribution: None,
        fmv_recorded: false,
        tax_basis_723: None,
        depreciable_basis: None,
        built_in_gain_704c: None,
        sec704c_1c: false,
        sec704c_method: None,
        basis_source: None,
    }
}

/// Bought: depreciable basis is cost.
fn bought(mut r: AssetRegisterRow) -> AssetRegisterRow {
    r.basis_rule = Some(BasisRule::Purchase);
    r.depreciable_basis = Some(r.cost);
    r
}

/// Contributed under `rule` with the member's adjusted basis `basis`
/// (cents) and its value `fmv` (cents; None: recorded before P3, read as
/// the book cost); figures as ledger_core::kernel::basis_figures gives
/// them.
fn contributed(
    mut r: AssetRegisterRow,
    rule: BasisRule,
    basis: i64,
    fmv: Option<i64>,
) -> AssetRegisterRow {
    let b = cents(basis);
    let v = fmv.map_or(r.cost, cents);
    let f = ledger_core::kernel::basis_figures(rule, r.cost, b, v).unwrap();
    r.basis_rule = Some(rule);
    r.contributor_adjusted_basis = Some(b);
    r.fmv_at_contribution = Some(v);
    r.fmv_recorded = fmv.is_some();
    r.tax_basis_723 = f.tax_basis_723;
    r.depreciable_basis = Some(f.depreciable_basis);
    r.built_in_gain_704c = f.built_in_gain_704c;
    r.sec704c_1c = f.built_in_gain_704c.is_some_and(|g| g < Decimal::ZERO);
    r
}

fn register(rows: Vec<AssetRegisterRow>) -> AssetRegister {
    let held = || rows.iter().filter(|r| r.disposed.is_none());
    let total_cost: Decimal = held().map(|r| r.cost).sum();
    let total_accumulated_scheduled: Decimal = held().map(|r| r.accumulated_scheduled).sum();
    let total_accumulated_posted: Decimal = held().map(|r| r.accumulated_posted).sum();
    AssetRegister {
        as_of: d(2026, 10, 15),
        total_depreciable_basis: held().filter_map(|r| r.depreciable_basis).sum(),
        total_tax_basis_723: held().filter_map(|r| r.tax_basis_723).sum(),
        missing_basis: held().filter(|r| r.depreciable_basis.is_none()).count(),
        total_book_value: total_cost - total_accumulated_posted,
        ledger_cost: total_cost,
        ledger_accumulated: total_accumulated_scheduled - cents(1234),
        total_cost,
        total_accumulated_scheduled,
        total_accumulated_posted,
        rows,
    }
}

/// Hand-worked: held cost 2,500.00 + 1,299.00 + 1,750.00 + 800.00 =
/// 6,349.00; scheduled 527.78 + 324.75 = 852.53; posted 500.00 + 324.75 =
/// 824.75; book value 6,349.00 − 824.75 = 5,524.25; §723 basis 900.00 +
/// 2,000.00 = 2,900.00; depreciable basis 2,500.00 + 900.00 + 1,750.00 =
/// 5,150.00; the books carry 852.53 − 12.34 = 840.19 accumulated, so
/// 12.34 isn't yet posted and 840.19 − 824.75 = 15.44 isn't from runs.
fn sample() -> AssetRegister {
    let mut disposed = row(3, "FL-0003", "Old monitor", 40000, 0, 0);
    disposed.book_value = Decimal::ZERO;
    disposed.disposed = Some((d(2026, 9, 30), cents(4500)));
    let mut laptop = row(
        1,
        "FL-0001",
        "Framework Laptop 13 (Ryzen 7 7840U, 64 GB, 4 TB)",
        250000,
        52778,
        50000,
    );
    laptop.serial = Some("FRANBMCP0A1234".into());
    laptop.salvage = cents(10000);
    laptop.acquired_date = d(2026, 1, 10);
    // A member's business laptop: carryover basis, built-in gain, its
    // value recorded, the traditional method chosen, the facts' source.
    let mut air = contributed(
        row(2, "FL-0002", "MacBook Air", 129900, 32475, 32475),
        BasisRule::Carryover723,
        90000,
        Some(129900),
    );
    air.sec704c_method = Some(Sec704cMethod::Traditional);
    air.basis_source = Some(ledger_core::settings::Source {
        decided_by: "Morgan Ellis".into(),
        decided_on: d(2026, 10, 20),
        citation: Some("entry 7".into()),
        note: None,
    });
    register(vec![
        bought(laptop),
        air,
        bought(disposed),
        // A member's personal camera, recorded before P3 (value read as
        // the book cost): worth less than its basis, so the depreciable
        // basis is the value and there's a built-in loss.
        contributed(
            row(4, "FL-0004", "Sony A7 IV camera", 175000, 0, 0),
            BasisRule::PersonalUseConversion,
            200000,
            None,
        ),
        // No tax basis recorded yet.
        row(5, "FL-0005", "Standing desk", 80000, 0, 0),
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

/// INVARIANTS 1, 2, 3, 5: header, every asset with its acquired and
/// in-service dates, the status of each, depreciation scheduled and
/// posted, the register's totals, and the reconciliation with its three
/// differences.
#[test]
fn pdf_shows_assets_totals_and_reconciliation() {
    let t = all_text(&sample(), &meta());
    for s in [
        "Fernhill Labs LLC",
        "Fixed Asset Register",
        "As of October 15, 2026",
        "ACQUIRED",
        "IN SERVICE",
        "DEPRECIATION SCHEDULED",
        "DEPRECIATION POSTED",
        "FL-0001",
        "FRANBMCP0A1234",
        "2026-01-10 2026-01-15",
        "MacBook Air",
        "Active",
        "Disposed 2026-09-30, proceeds 45.00",
        "Total, assets held",
        "$6,349.00 $852.53 $824.75 $5,524.25",
        "Cost per register",
        "Cost per books",
        "Difference (unregistered cost)",
        "Accumulated depreciation per schedule",
        "Accumulated depreciation per books",
        "Difference (not yet posted) $12.34",
        "Accumulated depreciation from the assets' runs",
        "Difference (not from runs) $15.44",
        "Journal seq 314",
    ] {
        assert!(t.contains(s), "{s:?} missing from {t:?}");
    }
    // The disposed asset's cost (400.00) is shown but in no total.
    assert!(t.contains("400.00"));
    let l = layout(&sample());
    assert_eq!(
        l.totals,
        [cents(634900), cents(85253), cents(82475), cents(552425)]
    );
}

/// INVARIANTS 7-9: the tax table shows each rule in short words, the
/// value at contribution (marked when read as the book cost), the
/// contributor's adjusted basis, the §723 basis, the depreciable basis,
/// the built-in gain and loss, the method and the facts' source under the
/// description, the register's tax totals, the missing line, and the
/// notes with their citations.
#[test]
fn pdf_shows_723_and_depreciable_basis() {
    let r = sample();
    let t = all_text(&r, &meta());
    for s in [
        "Federal tax figures, at contribution or purchase",
        "BASIS RULE",
        "VALUE AT CONTRIBUTION",
        "CONTRIBUTOR'S ADJUSTED BASIS",
        "§723 BASIS",
        "DEPRECIABLE BASIS",
        "BUILT-IN GAIN (LOSS)",
        // MacBook: cost, value, adjusted basis, §723, depreciable, gain.
        "Carryover 2 1,299.00 1,299.00 900.00 900.00 900.00 399.00",
        "Traditional (Treas. Reg. §1.704-3(b))",
        "Morgan Ellis, 2026-10-20: entry 7",
        // Camera: value read as cost (marked), basis 2,000.00, the lesser.
        "Personal-use conversion 3 1,750.00 * 1,750.00 2,000.00 2,000.00 1,750.00 (250.00)",
        "Standing desk Not recorded 800.00",
        "Total, assets held $2,900.00 $5,150.00",
        "Tax basis not recorded for 1 asset",
        "Notes",
        "Book value is cost less the depreciation the books carry",
        "IRC §1012",
        "IRC §723",
        "IRC §1016(a)",
        "Treas. Reg. §1.168(i)-4(b)",
        // The PDF wraps "§1.704-3(a)(3)(ii)" between its parentheses.
        "Treas. Reg. §1.704-3(a) (3)(ii)",
        "IRC §704(c)(1)(C)",
        "Value at contribution not recorded as a fact",
    ] {
        assert!(t.contains(s), "{s:?} missing from {t:?}");
    }
    let l = layout(&r);
    assert_eq!(
        l.tax_totals,
        [r.total_tax_basis_723, r.total_depreciable_basis]
    );
    assert_eq!(l.tax_totals, [cents(290000), cents(515000)]);
    assert_eq!(
        l.missing.as_deref(),
        Some("Tax basis not recorded for 1 asset")
    );
    assert_eq!(l.notes, notes());
    // Each line carries its row's tax fields unchanged.
    for (line, row) in l.lines.iter().zip(&r.rows) {
        assert_eq!(line.basis_rule, row.basis_rule);
        assert_eq!(line.fmv_at_contribution, row.fmv_at_contribution);
        assert_eq!(line.fmv_recorded, row.fmv_recorded);
        assert_eq!(
            line.contributor_adjusted_basis,
            row.contributor_adjusted_basis
        );
        assert_eq!(line.tax_basis_723, row.tax_basis_723);
        assert_eq!(line.depreciable_basis, row.depreciable_basis);
        assert_eq!(line.built_in_gain_704c, row.built_in_gain_704c);
        assert_eq!(line.sec704c_method, row.sec704c_method.map(method_cited));
        assert_eq!(line.accumulated_posted, row.accumulated_posted);
        assert_eq!(line.accumulated_scheduled, row.accumulated_scheduled);
    }
}

/// INVARIANT 8: no missing line when every asset held has a tax basis
/// (a disposed one without doesn't count), and the plural reads right.
#[test]
fn missing_line_only_when_missing() {
    let mut gone = row(1, "FL-0001", "Old monitor", 40000, 0, 0);
    gone.book_value = Decimal::ZERO;
    gone.disposed = Some((d(2026, 9, 30), cents(4500)));
    let r = register(vec![bought(row(2, "FL-0002", "Desk", 50000, 0, 0)), gone]);
    assert_eq!(r.missing_basis, 0);
    assert_eq!(layout(&r).missing, None);
    assert!(!all_text(&r, &meta()).contains("Tax basis not recorded"));
    assert_eq!(missing_line(0), None);
    assert_eq!(
        missing_line(2).as_deref(),
        Some("Tax basis not recorded for 2 assets")
    );
}

/// INVARIANT 7: the rules in words, short and with their citations; the
/// methods with their paragraphs.
#[test]
fn rule_words() {
    use BasisRule::*;
    assert_eq!(
        [Purchase, Carryover723, PersonalUseConversion].map(rule_label),
        ["Purchase", "Carryover", "Personal-use conversion"]
    );
    assert_eq!(
        [Purchase, Carryover723, PersonalUseConversion].map(rule_cited),
        [
            "Purchase (IRC §1012)",
            "Carryover (IRC §723)",
            "Personal-use conversion (Treas. Reg. §1.168(i)-4(b))"
        ]
    );
    assert_eq!(
        Sec704cMethod::ALL.map(method_cited),
        [
            "Traditional (Treas. Reg. §1.704-3(b))".to_string(),
            "Traditional with curative allocations (Treas. Reg. §1.704-3(c))".to_string(),
            "Remedial (Treas. Reg. §1.704-3(d))".to_string(),
        ]
    );
}

/// INVARIANT 9: the notes number the rules 1, 2, 3 (the general note
/// first and unnumbered, the built-in gain note 4, the value read as cost
/// "*"), each rule's note cites that rule, and the PDF marks each
/// recorded line's rule with its note's number; a line with no rule
/// recorded has no mark.
#[test]
fn rule_marks_match_their_notes() {
    use BasisRule::*;
    let rules = [Purchase, Carryover723, PersonalUseConversion];
    assert_eq!(rules.map(rule_mark), ["1", "2", "3"]);
    let n = notes();
    let marks: Vec<&str> = n.iter().map(|n| n.mark.as_str()).collect();
    assert_eq!(marks, ["", "1", "2", "3", "4", FMV_NOT_RECORDED_MARK]);
    for rule in rules {
        let note = n.iter().find(|n| n.mark == rule_mark(rule)).unwrap();
        let cites = format!("{}: ", rule_cited(rule));
        assert!(note.text.starts_with(&cites), "{rule:?}: {:?}", note.text);
    }
    // The template's input: each line its rule's mark, in register order.
    let r = sample();
    let data = template_data(&r, &meta());
    let marks: Vec<&str> = data["lines"]
        .as_array()
        .unwrap()
        .iter()
        .map(|l| l["rule_mark"].as_str().unwrap())
        .collect();
    assert_eq!(marks, ["1", "2", "1", "3", ""]);
    // The PDF: the mark right after the rule, the note under its number.
    let t = all_text(&r, &meta());
    for s in [
        "(Ryzen 7 7840U, 64 GB, 4 TB) Purchase 1 2,500.00",
        "Carryover 2 1,299.00",
        "Old monitor Purchase 1 400.00",
        "Sony A7 IV camera Personal-use conversion 3 1,750.00",
        "Standing desk Not recorded 800.00",
        "1 Purchase (IRC §1012): property the company bought",
        "2 Carryover (IRC §723): the company's basis in property a member contributes",
        "3 Personal-use conversion (Treas. Reg. §1.168(i)-4(b)): the §723 basis",
        "4 Built-in gain (loss), IRC §704(c):",
    ] {
        assert!(t.contains(s), "{s:?} missing from {t:?}");
    }
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
        "Difference (not from runs)",
    ] {
        assert!(t.contains(s), "{s:?} missing from {t:?}");
    }
}

/// INVARIANT 6: markup in every text field prints literally and never runs.
#[test]
fn text_is_data() {
    let mut a = row(1, "#FL-*1*", "#panic() *bold* $x$ <l> @r [x] \\", 100, 0, 0);
    a.serial = Some("_u_ // /*".into());
    let m = ReportMeta {
        company: "#co *Inc*".into(),
        ..meta()
    };
    let t = all_text(&register(vec![a]), &m);
    for s in [
        "#FL-*1*", "#panic()", "*bold*", "$x$", "<l>", "@r", "[x]", "_u_", "#co", "*Inc*",
    ] {
        assert!(t.contains(s), "{s:?} not literal in {t:?}");
    }
    for s in [
        "#panic()", "*", "_", "$", "[", "]", "\\", "#", "<a>", "@a", "//", "/*", "\"", "`",
    ] {
        let mut a = row(1, s, s, 100, 0, 0);
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
                &format!("FL-{i:04}"),
                &format!("Asset {i}"),
                100_000,
                1_000,
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
