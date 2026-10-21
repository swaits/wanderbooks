//! The fixed asset register as XLSX and PDF, for the CPA.
//!
//! [`ledger_core::fixed_asset_register`] computes the register; [`layout`]
//! lays it out once — columns, one line per asset, the totals, and the
//! reconciliation to the general ledger — and both renderers draw from
//! that layout, so they can't drift apart. Amounts stay exact `Decimal`s
//! until a renderer writes them, through the same paths as the statements
//! ([`crate::report::amount_parts`] for PDF, the accounting number formats
//! of [`crate::xlsx`] for XLSX).
//!
//! ```text
//! GOAL: one list of what the company owns, its cost, depreciation due and
//!       posted, and book value, with its tax figures, that a CPA can
//!       file, and that shows at a glance whether the register and the
//!       books agree.
//! SOURCE: ledger_core::fixed_asset_register (INVARIANTS 1-6); CONVENTION
//!         (layout, the statements' header and footer).
//! INVARIANTS:
//!   1. One line per register row, in the register's order, disposed ones
//!      included and marked "Disposed DATE, proceeds AMOUNT"; held ones
//!      "Active". Each shows its acquired and in-service dates.
//!   2. Accumulated depreciation twice, never confused: scheduled (due by
//!      the date) and posted (what the books carry); book value is the
//!      posted figure's. The totals are the register's own (held assets
//!      only), never re-summed here.
//!   3. Reconciliation: cost per register, per books, and their difference
//!      (unregistered_cost); accumulated depreciation per schedule, per
//!      books, and their difference (unposted_depreciation); from the
//!      assets' runs, and the books' difference from it
//!      (unattributed_accumulated), exactly.
//!   4. An empty register says "No fixed assets" and still shows totals
//!      and the reconciliation (the books may carry unregistered cost).
//!   5. Header and footer as on every statement: company, "Fixed Asset
//!      Register", "As of DATE"; basis, prepared on, journal seq.
//!   6. PDF: deterministic bytes; every text is data, never markup.
//!   7. Tax figures beside book value: every line carries its basis rule
//!      (in words: [`rule_label`], [`rule_cited`]), the value at
//!      contribution (marked [`FMV_NOT_RECORDED_MARK`] when read as the
//!      book cost), the contributor's adjusted basis, the §723 basis, the
//!      depreciable basis, the §704(c) built-in gain (loss), the §704(c)
//!      method, and who gave the facts, each the register row's own
//!      (ledger_core::Asset::basis_figures), never re-derived here; a line
//!      with none recorded says "Not recorded" and leaves them blank.
//!   8. The tax totals are the register's total_tax_basis_723 and
//!      total_depreciable_basis; when missing_basis > 0 a line says "Tax
//!      basis not recorded for N asset(s)" (see [`missing_line`]).
//!   9. The notes ([`notes`]) say what book value and the scheduled figure
//!      are, that the tax figures are at contribution, and cite each rule
//!      (IRC §1012(a), §723, §1016(a), Treas. Reg. §1.168(i)-4(b),
//!      §1.704-3(a)(3)(ii), IRC §704(c)(1)(C)). The PDF marks each rule in
//!      the tax table with its note's number.
//! ```
//!
//! SOURCE (7-9): docs/spec/rules.md B-25;
//! docs/spec/sources/irs-partnership-basis.md.

use crate::pdf::PdfError;
use crate::report::{ReportMeta, amount_parts, format_amount, long_date};
use crate::xlsx::{Formats, XlsxError, footer, header, number};
use ledger_core::reports::{AssetRegister, AssetRegisterRow};
use ledger_core::{BasisRule, Sec704cMethod};
use rust_decimal::Decimal;
use rust_xlsxwriter::{Format, FormatAlign, Workbook};
use std::sync::OnceLock;
use typst::foundations::{Dict, IntoValue};
use typst_as_lib::{TypstEngine, TypstTemplateMainFile};
use typst_layout::PagedDocument;

/// The report's title.
pub const TITLE: &str = "Fixed Asset Register";

/// Column headings, left to right.
pub const COLUMNS: [&str; 12] = [
    "Tag",
    "Description",
    "Serial",
    "Acquired",
    "In service",
    "Cost",
    "Salvage",
    "Life (months)",
    "Depreciation scheduled",
    "Depreciation posted",
    "Book value",
    "Status",
];

/// The PDF's tax table's headings, left to right.
pub const TAX_COLUMNS: [&str; 9] = [
    "Tag",
    "Description",
    "Basis rule",
    "Book cost",
    "Value at contribution",
    "Contributor's adjusted basis",
    "§723 basis",
    "Depreciable basis",
    "Built-in gain (loss)",
];

/// The XLSX headings, left to right: the book columns, the tax columns,
/// then the status.
pub const XLSX_COLUMNS: [&str; 21] = [
    "Tag",
    "Description",
    "Serial",
    "Acquired",
    "In service",
    "Cost",
    "Salvage",
    "Life (months)",
    "Depreciation scheduled",
    "Depreciation posted",
    "Book value",
    "Basis rule",
    "Value at contribution",
    "Contributor's adjusted basis",
    "§723 basis",
    "Depreciable basis",
    "Built-in gain (loss)",
    "Allocation method",
    "Basis facts from",
    "Value recorded",
    "Status",
];

/// The line shown when there are no assets.
pub const EMPTY: &str = "No fixed assets";

/// The tax table's title.
pub const TAX_TITLE: &str = "Federal tax figures, at contribution or purchase";

/// The rule cell of an asset with no tax basis recorded.
pub const NOT_RECORDED: &str = "Not recorded";

/// The mark on a value at contribution read as the book cost (recorded
/// before the value was a fact): its note is [`notes`]' last.
pub const FMV_NOT_RECORDED_MARK: &str = "*";

/// A rule in short words, for the table: "Purchase", "Carryover",
/// "Personal-use conversion".
pub fn rule_label(r: BasisRule) -> &'static str {
    match r {
        BasisRule::Purchase => "Purchase",
        BasisRule::Carryover723 => "Carryover",
        BasisRule::PersonalUseConversion => "Personal-use conversion",
    }
}

/// A rule in words with its citation: "Purchase (IRC §1012)", "Carryover
/// (IRC §723)", "Personal-use conversion (Treas. Reg. §1.168(i)-4(b))".
pub fn rule_cited(r: BasisRule) -> &'static str {
    match r {
        BasisRule::Purchase => "Purchase (IRC §1012)",
        BasisRule::Carryover723 => "Carryover (IRC §723)",
        BasisRule::PersonalUseConversion => "Personal-use conversion (Treas. Reg. §1.168(i)-4(b))",
    }
}

/// The number of a rule's note in [`notes`].
pub fn rule_mark(r: BasisRule) -> &'static str {
    match r {
        BasisRule::Purchase => "1",
        BasisRule::Carryover723 => "2",
        BasisRule::PersonalUseConversion => "3",
    }
}

/// A §704(c) method in words with its paragraph: "Traditional (Treas.
/// Reg. §1.704-3(b))", ...
pub fn method_cited(m: Sec704cMethod) -> String {
    let words = match m {
        Sec704cMethod::Traditional => "Traditional",
        Sec704cMethod::TraditionalWithCurative => "Traditional with curative allocations",
        Sec704cMethod::Remedial => "Remedial",
    };
    format!("{words} ({})", m.citation())
}

/// One note under the register.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Note {
    /// Its number, as the tax table marks it, or empty for a general note.
    pub mark: String,
    /// What it says.
    pub text: String,
}

/// The notes (INVARIANT 9): book value and the scheduled figure, the tax
/// figures, each rule's citation, the built-in gain (loss), and a value
/// read as the book cost.
pub fn notes() -> Vec<Note> {
    let n = |mark: &str, text: &str| Note {
        mark: mark.to_string(),
        text: text.to_string(),
    };
    vec![
        n(
            "",
            "Book value is cost less the depreciation the books carry (posted); scheduled is what \
             the depreciation schedule says is due by the date, and the difference is depreciation \
             not yet run. Property a member contributed is on the books at its fair market value \
             at contribution, property the company bought at its price. Depreciation here is book \
             depreciation. The tax figures are at contribution (or purchase): tax depreciation, \
             and what remains of a built-in gain or loss later, are the CPA's.",
        ),
        n(
            rule_mark(BasisRule::Purchase),
            "Purchase (IRC §1012): property the company bought has a basis of its cost, which tax \
             depreciation starts from.",
        ),
        n(
            rule_mark(BasisRule::Carryover723),
            "Carryover (IRC §723): the company's basis in property a member contributes is the \
             member's adjusted basis at contribution (cost plus improvements, less depreciation \
             allowed or allowable, IRC §1016(a)); tax depreciation starts from it.",
        ),
        n(
            rule_mark(BasisRule::PersonalUseConversion),
            "Personal-use conversion (Treas. Reg. §1.168(i)-4(b)): the §723 basis as for \
             carryover; tax depreciation starts from the lesser of its fair market value and that \
             basis at conversion.",
        ),
        n(
            "4",
            "Built-in gain (loss), IRC §704(c): fair market value less the §723 basis at \
             contribution (Treas. Reg. §1.704-3(a)(3)(ii)); a built-in loss counts only for the \
             contributing member (IRC §704(c)(1)(C)). The method chosen allocates it among the \
             members (Treas. Reg. §1.704-3(b)-(d)).",
        ),
        n(
            FMV_NOT_RECORDED_MARK,
            "Value at contribution not recorded as a fact: read as the book cost, as it was \
             recorded before the value was asked for.",
        ),
    ]
}

/// "Tax basis not recorded for 1 asset" / "... for 2 assets", or `None`
/// when every asset held has one.
pub fn missing_line(missing: usize) -> Option<String> {
    (missing > 0).then(|| {
        format!(
            "Tax basis not recorded for {missing} {}",
            if missing == 1 { "asset" } else { "assets" }
        )
    })
}

/// One asset's line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AssetLine {
    /// Tag, or empty.
    pub tag: String,
    /// What it is.
    pub description: String,
    /// Serial number, or empty.
    pub serial: String,
    /// Acquired date, ISO 8601.
    pub acquired: String,
    /// In-service date, ISO 8601.
    pub in_service: String,
    /// Capitalized cost.
    pub cost: Decimal,
    /// Salvage value.
    pub salvage: Decimal,
    /// Useful life in months.
    pub life_months: u32,
    /// Accumulated depreciation by schedule (zero once disposed).
    pub accumulated_scheduled: Decimal,
    /// Accumulated depreciation the books carry (zero once disposed).
    pub accumulated_posted: Decimal,
    /// Book value: cost less posted (zero once disposed).
    pub book_value: Decimal,
    /// "Active", or "Disposed 2026-09-30, proceeds 450.00".
    pub status: String,
    /// Disposed by the date (shown, not in the totals).
    pub disposed: bool,
    /// The basis rule, if recorded.
    pub basis_rule: Option<BasisRule>,
    /// Fair market value at contribution (a contribution only).
    pub fmv_at_contribution: Option<Decimal>,
    /// Whether that value was recorded (false: read as the book cost).
    pub fmv_recorded: bool,
    /// The contributing member's adjusted basis (a contribution only).
    pub contributor_adjusted_basis: Option<Decimal>,
    /// The §723 basis (a contribution only).
    pub tax_basis_723: Option<Decimal>,
    /// The depreciable tax basis, if recorded.
    pub depreciable_basis: Option<Decimal>,
    /// Built-in gain (+) or loss (−) at contribution, if any.
    pub built_in_gain_704c: Option<Decimal>,
    /// The §704(c) method, in words with its paragraph, if chosen.
    pub sec704c_method: Option<String>,
    /// Who gave the basis facts, when, and on what, if recorded.
    pub basis_source: Option<String>,
}

/// Which register column a reconciliation amount sits under.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReconColumn {
    /// Under "Cost".
    Cost,
    /// Under the accumulated depreciation columns.
    Accumulated,
}

/// One reconciliation line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReconLine {
    /// "Cost per register", ...
    pub label: String,
    /// The amount.
    pub amount: Decimal,
    /// The column it reconciles.
    pub column: ReconColumn,
    /// A difference line (ruled above, bold).
    pub difference: bool,
}

/// The register laid out for rendering.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RegisterLayout {
    /// [`TITLE`].
    pub title: String,
    /// "As of October 15, 2026".
    pub period: String,
    /// One per asset, in register order.
    pub lines: Vec<AssetLine>,
    /// Total cost, accumulated depreciation scheduled, posted, and book
    /// value of the assets held.
    pub totals: [Decimal; 4],
    /// Total §723 basis and depreciable basis of the assets held.
    pub tax_totals: [Decimal; 2],
    /// [`missing_line`] of the register's missing_basis.
    pub missing: Option<String>,
    /// [`notes`].
    pub notes: Vec<Note>,
    /// Cost, then accumulated depreciation: per register, per books,
    /// differences.
    pub reconciliation: Vec<ReconLine>,
}

fn status(a: &AssetRegisterRow) -> String {
    match a.disposed {
        Some((date, proceeds)) => {
            format!("Disposed {date}, proceeds {}", format_amount(proceeds))
        }
        None => "Active".to_string(),
    }
}

/// Who gave an asset's basis facts: "Morgan Ellis, 2026-10-20: entry 14".
pub fn source_text(s: &ledger_core::settings::Source) -> String {
    let mut t = format!("{}, {}", s.decided_by, s.decided_on);
    if let Some(c) = &s.citation {
        t.push_str(&format!(": {c}"));
    }
    t
}

/// Lay the register out (see the module spec).
pub fn layout(r: &AssetRegister) -> RegisterLayout {
    let recon = |label: &str, amount, column, difference| ReconLine {
        label: label.to_string(),
        amount,
        column,
        difference,
    };
    RegisterLayout {
        title: TITLE.to_string(),
        period: format!("As of {}", long_date(r.as_of)),
        lines: r
            .rows
            .iter()
            .map(|a| AssetLine {
                tag: a.tag.clone().unwrap_or_default(),
                description: a.description.clone(),
                serial: a.serial.clone().unwrap_or_default(),
                acquired: a.acquired_date.to_string(),
                in_service: a.in_service_date.to_string(),
                cost: a.cost,
                salvage: a.salvage,
                life_months: a.useful_life_months,
                accumulated_scheduled: a.accumulated_scheduled,
                accumulated_posted: a.accumulated_posted,
                book_value: a.book_value,
                status: status(a),
                disposed: a.disposed.is_some(),
                basis_rule: a.basis_rule,
                fmv_at_contribution: a.fmv_at_contribution,
                fmv_recorded: a.fmv_recorded,
                contributor_adjusted_basis: a.contributor_adjusted_basis,
                tax_basis_723: a.tax_basis_723,
                depreciable_basis: a.depreciable_basis,
                built_in_gain_704c: a.built_in_gain_704c,
                sec704c_method: a.sec704c_method.map(method_cited),
                basis_source: a.basis_source.as_ref().map(source_text),
            })
            .collect(),
        totals: [
            r.total_cost,
            r.total_accumulated_scheduled,
            r.total_accumulated_posted,
            r.total_book_value,
        ],
        tax_totals: [r.total_tax_basis_723, r.total_depreciable_basis],
        missing: missing_line(r.missing_basis),
        notes: notes(),
        reconciliation: vec![
            recon("Cost per register", r.total_cost, ReconColumn::Cost, false),
            recon("Cost per books", r.ledger_cost, ReconColumn::Cost, false),
            recon(
                "Difference (unregistered cost)",
                r.unregistered_cost(),
                ReconColumn::Cost,
                true,
            ),
            recon(
                "Accumulated depreciation per schedule",
                r.total_accumulated_scheduled,
                ReconColumn::Accumulated,
                false,
            ),
            recon(
                "Accumulated depreciation per books",
                r.ledger_accumulated,
                ReconColumn::Accumulated,
                false,
            ),
            recon(
                "Difference (not yet posted)",
                r.unposted_depreciation(),
                ReconColumn::Accumulated,
                true,
            ),
            recon(
                "Accumulated depreciation from the assets' runs",
                r.total_accumulated_posted,
                ReconColumn::Accumulated,
                false,
            ),
            recon(
                "Accumulated depreciation per books",
                r.ledger_accumulated,
                ReconColumn::Accumulated,
                false,
            ),
            recon(
                "Difference (not from runs)",
                r.unattributed_accumulated(),
                ReconColumn::Accumulated,
                true,
            ),
        ],
    }
}

/// The fixed asset register as XLSX bytes.
///
/// Layout as every statement (crate::xlsx INVARIANT 4): company, title,
/// and date on rows 1-3, headings ([`XLSX_COLUMNS`]) on row 5, one row
/// per asset from row 6, amounts as numbers in the accounting format, the
/// basis rule and §704(c) method in words with their citations
/// ([`rule_cited`], [`method_cited`]); then the totals row (double rule)
/// with the §723 and depreciable basis totals under theirs, the missing
/// basis line when there is one, an empty row, "Reconciliation to the
/// books" and its lines with amounts under Cost and Depreciation
/// scheduled, an empty row, and "Notes" with [`notes`].
pub fn asset_register_xlsx(r: &AssetRegister, meta: &ReportMeta) -> Result<Vec<u8>, XlsxError> {
    const AMOUNTS: [u16; 10] = [5, 6, 8, 9, 10, 12, 13, 14, 15, 16];
    const TOTALS: [u16; 6] = [5, 8, 9, 10, 14, 15];
    let l = layout(r);
    let mut workbook = Workbook::new();
    let sheet = workbook.add_worksheet();
    let f = Formats::new();
    sheet.set_name(TITLE)?;
    let width = XLSX_COLUMNS.len() as u16;
    let mut row = header(sheet, &f, meta, &l.title, &l.period, width)?;
    for (i, h) in XLSX_COLUMNS.iter().enumerate() {
        let fmt = if AMOUNTS.contains(&(i as u16)) || i == 7 {
            &f.header_right
        } else {
            &f.header
        };
        sheet.write_string_with_format(row, i as u16, h.to_uppercase(), fmt)?;
    }
    row += 1;
    if l.lines.is_empty() {
        sheet.write_string_with_format(row, 0, EMPTY, &f.sub_label)?;
        row += 1;
    }
    let quiet = Format::new().set_italic().set_font_color("#666666");
    let quiet_plain = f.plain.clone().set_italic().set_font_color("#666666");
    let missing = Format::new().set_italic().set_font_color("#9A5B00");
    for a in &l.lines {
        let (text, amount) = if a.disposed {
            (&quiet, &quiet_plain)
        } else {
            (&Format::new(), &f.plain)
        };
        sheet.write_string_with_format(row, 0, &a.tag, text)?;
        sheet.write_string_with_format(row, 1, &a.description, text)?;
        sheet.write_string_with_format(row, 2, &a.serial, text)?;
        sheet.write_string_with_format(row, 3, &a.acquired, text)?;
        sheet.write_string_with_format(row, 4, &a.in_service, text)?;
        number(sheet, row, 5, a.cost, amount)?;
        number(sheet, row, 6, a.salvage, amount)?;
        sheet.write_number_with_format(
            row,
            7,
            f64::from(a.life_months),
            &text.clone().set_align(FormatAlign::Right),
        )?;
        number(sheet, row, 8, a.accumulated_scheduled, amount)?;
        number(sheet, row, 9, a.accumulated_posted, amount)?;
        number(sheet, row, 10, a.book_value, amount)?;
        match a.basis_rule {
            Some(rule) => sheet.write_string_with_format(row, 11, rule_cited(rule), text)?,
            None => sheet.write_string_with_format(
                row,
                11,
                NOT_RECORDED,
                if a.disposed { text } else { &missing },
            )?,
        };
        for (c, v) in [
            (12, a.fmv_at_contribution),
            (13, a.contributor_adjusted_basis),
            (14, a.tax_basis_723),
            (15, a.depreciable_basis),
            (16, a.built_in_gain_704c),
        ] {
            if let Some(v) = v {
                number(sheet, row, c, v, amount)?;
            }
        }
        if let Some(m) = &a.sec704c_method {
            sheet.write_string_with_format(row, 17, m, text)?;
        }
        if let Some(src) = &a.basis_source {
            sheet.write_string_with_format(row, 18, src, text)?;
        }
        if a.fmv_at_contribution.is_some() {
            sheet.write_string_with_format(
                row,
                19,
                if a.fmv_recorded {
                    "Yes"
                } else {
                    "No: read as the book cost"
                },
                text,
            )?;
        }
        sheet.write_string_with_format(row, 20, &a.status, text)?;
        row += 1;
    }
    sheet.write_string_with_format(row, 0, "Total, assets held", &f.bold)?;
    for c in (1..width).filter(|c| !TOTALS.contains(c)) {
        sheet.write_blank(row, c, &f.dollars_total)?;
    }
    for (c, d) in TOTALS
        .into_iter()
        .zip(l.totals.into_iter().chain(l.tax_totals))
    {
        number(sheet, row, c, d, &f.dollars_total)?;
    }
    row += 1;
    if let Some(m) = &l.missing {
        sheet.write_string_with_format(row, 0, m, &f.sub_label)?;
        row += 1;
    }
    row += 1;
    sheet.write_string_with_format(row, 0, "Reconciliation to the books", &f.bold)?;
    row += 1;
    for line in &l.reconciliation {
        let col = match line.column {
            ReconColumn::Cost => 5,
            ReconColumn::Accumulated => 8,
        };
        let (label_fmt, amount_fmt) = if line.difference {
            (&f.bold, &f.dollars_sum)
        } else {
            (&f.line_label, &f.plain)
        };
        sheet.write_string_with_format(row, 0, &line.label, label_fmt)?;
        number(sheet, row, col, line.amount, amount_fmt)?;
        row += 1;
        if line.difference {
            row += 1;
        }
    }
    sheet.write_string_with_format(row, 0, "Notes", &f.bold)?;
    row += 1;
    for n in &l.notes {
        let text = if n.mark.is_empty() {
            n.text.clone()
        } else {
            format!("{}. {}", n.mark, n.text)
        };
        sheet.write_string_with_format(row, 0, text, &f.line_label)?;
        row += 1;
    }
    for (c, w) in [
        (0, 12.0),
        (1, 40.0),
        (2, 20.0),
        (3, 12.0),
        (4, 12.0),
        (5, 14.0),
        (6, 12.0),
        (7, 9.0),
        (8, 16.0),
        (9, 16.0),
        (10, 14.0),
        (11, 30.0),
        (12, 14.0),
        (13, 16.0),
        (14, 14.0),
        (15, 14.0),
        (16, 16.0),
        (17, 30.0),
        (18, 30.0),
        (19, 16.0),
        (20, 34.0),
    ] {
        sheet.set_column_width(c, w)?;
    }
    footer(sheet, &f, meta, row, width)?;
    Ok(workbook.save_to_buffer()?)
}

static TEMPLATE: &str = include_str!("../templates/asset_register.typ");

fn engine() -> &'static TypstEngine<TypstTemplateMainFile> {
    static ENGINE: OnceLock<TypstEngine<TypstTemplateMainFile>> = OnceLock::new();
    ENGINE.get_or_init(|| {
        TypstEngine::builder()
            .main_file(TEMPLATE)
            .fonts(crate::fonts::typst_fonts())
            .build()
    })
}

fn amount(d: Decimal) -> serde_json::Value {
    let p = amount_parts(d);
    serde_json::json!({ "digits": p.digits, "negative": p.negative })
}

fn maybe(d: Option<Decimal>) -> serde_json::Value {
    d.map_or(serde_json::Value::Null, amount)
}

/// The PDF template's input: the layout, every amount split for
/// alignment.
pub fn template_data(r: &AssetRegister, meta: &ReportMeta) -> serde_json::Value {
    let l = layout(r);
    serde_json::json!({
        "company": meta.company,
        "title": l.title,
        "period": l.period,
        "footer": meta.footer(),
        "columns": COLUMNS,
        "tax_title": TAX_TITLE,
        "tax_columns": TAX_COLUMNS,
        "empty": EMPTY,
        "fmv_mark": FMV_NOT_RECORDED_MARK,
        "lines": l.lines.iter().map(|a| serde_json::json!({
            "tag": a.tag,
            "description": a.description,
            "serial": a.serial,
            "acquired": a.acquired,
            "in_service": a.in_service,
            "cost": amount(a.cost),
            "salvage": amount(a.salvage),
            "life": a.life_months.to_string(),
            "accumulated_scheduled": amount(a.accumulated_scheduled),
            "accumulated_posted": amount(a.accumulated_posted),
            "book_value": amount(a.book_value),
            "status": a.status,
            "disposed": a.disposed,
            "rule": a.basis_rule.map_or(NOT_RECORDED, rule_label),
            "rule_mark": a.basis_rule.map_or("", rule_mark),
            "recorded": a.basis_rule.is_some(),
            "fmv_at_contribution": maybe(a.fmv_at_contribution),
            "fmv_recorded": a.fmv_recorded,
            "contributor_adjusted_basis": maybe(a.contributor_adjusted_basis),
            "tax_basis_723": maybe(a.tax_basis_723),
            "depreciable_basis": maybe(a.depreciable_basis),
            "built_in_gain": maybe(a.built_in_gain_704c),
            "method": a.sec704c_method,
            "source": a.basis_source,
        })).collect::<Vec<_>>(),
        "totals": l.totals.iter().map(|d| amount(*d)).collect::<Vec<_>>(),
        "tax_totals": l.tax_totals.iter().map(|d| amount(*d)).collect::<Vec<_>>(),
        "missing": l.missing,
        "notes": l.notes.iter().map(|n| serde_json::json!({
            "mark": n.mark,
            "text": n.text,
        })).collect::<Vec<_>>(),
        "reconciliation": l.reconciliation.iter().map(|x| serde_json::json!({
            "label": x.label,
            "amount": amount(x.amount),
            "difference": x.difference,
        })).collect::<Vec<_>>(),
    })
}

/// Typeset the register into a document (pages, before PDF encoding), so
/// the layout can be inspected.
pub fn typeset_asset_register(
    r: &AssetRegister,
    meta: &ReportMeta,
) -> Result<PagedDocument, PdfError> {
    let mut inputs = Dict::new();
    inputs.insert(
        "data".into(),
        template_data(r, meta).to_string().into_value(),
    );
    engine()
        .compile_with_input(inputs)
        .output
        .map_err(|e| PdfError::Compile(format!("{e:?}")))
}

/// The fixed asset register as PDF bytes (US Letter, landscape). See the
/// module spec.
pub fn asset_register_pdf(r: &AssetRegister, meta: &ReportMeta) -> Result<Vec<u8>, PdfError> {
    let doc = typeset_asset_register(r, meta)?;
    typst_pdf::pdf(&doc, &typst_pdf::PdfOptions::default())
        .map_err(|e| PdfError::Save(format!("{e:?}")))
}
