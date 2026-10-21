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
//! GOAL: one list of what the company owns, its cost, depreciation, and
//!       book value, that a CPA can file, and that shows at a glance
//!       whether the register and the books agree.
//! SOURCE: ledger_core::fixed_asset_register (INVARIANTS 1-4); CONVENTION
//!         (layout, the statements' header and footer).
//! INVARIANTS:
//!   1. One line per register row, in the register's order, disposed ones
//!      included and marked "Disposed DATE, proceeds AMOUNT"; held ones
//!      "Active".
//!   2. The totals are the register's own totals (held assets only),
//!      never re-summed here.
//!   3. Reconciliation: cost per register, per books, and their difference
//!      (unregistered_cost); accumulated depreciation per schedule, per
//!      books, and their difference (unposted_depreciation), exactly.
//!   4. An empty register says "No fixed assets" and still shows totals
//!      and the reconciliation (the books may carry unregistered cost).
//!   5. Header and footer as on every statement: company, "Fixed Asset
//!      Register", "As of DATE"; basis, prepared on, journal seq.
//!   6. PDF: deterministic bytes; every text is data, never markup.
//!   7. Tax basis beside book value: every line carries its tax basis
//!      rule (in words: [`rule_label`], [`rule_cited`]), the contributor's
//!      basis, the tax basis, and the built-in gain (loss), each the
//!      register row's own (ledger_core::Asset::tax_basis,
//!      Asset::built_in_gain), never re-derived here; a line with none
//!      recorded says "Not recorded" and leaves the amounts blank.
//!   8. The tax basis total is the register's total_tax_basis; when
//!      missing_tax_basis > 0 a line says "Tax basis not recorded for N
//!      asset(s)" (see [`missing_line`]).
//!   9. The notes ([`notes`]) say that book value is fair market value at
//!      contribution and tax basis follows its rule, and cite each rule
//!      (IRC §1012(a), §723, Treas. Reg. §1.168(i)-4(b), IRC §704(c)).
//!      The PDF marks each rule in the tax table with its note's number.
//! ```
//!
//! SOURCE (7-9): docs/spec/rules.md B-25;
//! docs/spec/sources/irs-partnership-basis.md.

use crate::pdf::PdfError;
use crate::report::{ReportMeta, amount_parts, format_amount, long_date};
use crate::xlsx::{Formats, XlsxError, footer, header, number};
use ledger_core::TaxBasisRule;
use ledger_core::reports::{AssetRegister, AssetRegisterRow};
use rust_decimal::Decimal;
use rust_xlsxwriter::{Format, FormatAlign, Workbook};
use std::sync::OnceLock;
use typst::foundations::{Dict, IntoValue};
use typst_as_lib::{TypstEngine, TypstTemplateMainFile};
use typst_layout::PagedDocument;

/// The report's title.
pub const TITLE: &str = "Fixed Asset Register";

/// Column headings, left to right.
pub const COLUMNS: [&str; 10] = [
    "Tag",
    "Description",
    "Serial",
    "In service",
    "Cost",
    "Salvage",
    "Life (months)",
    "Accumulated depreciation",
    "Book value",
    "Status",
];

/// The PDF's tax basis table's headings, left to right.
pub const TAX_COLUMNS: [&str; 7] = [
    "Tag",
    "Description",
    "Tax basis rule",
    "Book cost",
    "Contributor's basis",
    "Tax basis",
    "Built-in gain (loss)",
];

/// The XLSX headings, left to right: the book columns, the tax columns,
/// then the status.
pub const XLSX_COLUMNS: [&str; 14] = [
    "Tag",
    "Description",
    "Serial",
    "In service",
    "Cost",
    "Salvage",
    "Life (months)",
    "Accumulated depreciation",
    "Book value",
    "Tax basis rule",
    "Contributor's basis",
    "Tax basis",
    "Built-in gain (loss)",
    "Status",
];

/// The line shown when there are no assets.
pub const EMPTY: &str = "No fixed assets";

/// The tax table's title.
pub const TAX_TITLE: &str = "Federal tax basis";

/// The rule cell of an asset with no tax basis recorded.
pub const NOT_RECORDED: &str = "Not recorded";

/// A rule in short words, for the table: "Cost", "Carryover",
/// "Personal-use conversion".
pub fn rule_label(r: TaxBasisRule) -> &'static str {
    match r {
        TaxBasisRule::Cost => "Cost",
        TaxBasisRule::Carryover => "Carryover",
        TaxBasisRule::PersonalUseConversion => "Personal-use conversion",
    }
}

/// A rule in words with its citation: "Cost", "Carryover (IRC §723)",
/// "Personal-use conversion (Treas. Reg. §1.168(i)-4(b))".
pub fn rule_cited(r: TaxBasisRule) -> &'static str {
    match r {
        TaxBasisRule::Cost => "Cost",
        TaxBasisRule::Carryover => "Carryover (IRC §723)",
        TaxBasisRule::PersonalUseConversion => {
            "Personal-use conversion (Treas. Reg. §1.168(i)-4(b))"
        }
    }
}

/// The number of a rule's note in [`notes`].
pub fn rule_mark(r: TaxBasisRule) -> &'static str {
    match r {
        TaxBasisRule::Cost => "1",
        TaxBasisRule::Carryover => "2",
        TaxBasisRule::PersonalUseConversion => "3",
    }
}

/// One note under the register.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Note {
    /// Its number, as the tax table marks it, or empty for a general note.
    pub mark: String,
    /// What it says.
    pub text: String,
}

/// The notes (INVARIANT 9): book value against tax basis, then each
/// rule's citation and the built-in gain (loss).
pub fn notes() -> Vec<Note> {
    let n = |mark: &str, text: &str| Note {
        mark: mark.to_string(),
        text: text.to_string(),
    };
    vec![
        n(
            "",
            "Book value is the fair market value at contribution for property a member contributed, \
             and the price paid for property the company bought; tax basis follows its rule and can \
             differ. Depreciation here is book depreciation; tax depreciation starts from the tax \
             basis and is the CPA's.",
        ),
        n(
            rule_mark(TaxBasisRule::Cost),
            "Cost: property the company bought has a basis of its cost (IRC §1012(a)).",
        ),
        n(
            rule_mark(TaxBasisRule::Carryover),
            "Carryover (IRC §723): property a member contributes keeps the member's adjusted \
             basis at contribution (their cost less any depreciation they took).",
        ),
        n(
            rule_mark(TaxBasisRule::PersonalUseConversion),
            "Personal-use conversion (Treas. Reg. §1.168(i)-4(b)): property converted from \
             personal use depreciates from the lesser of its fair market value and the member's \
             adjusted basis at conversion.",
        ),
        n(
            "4",
            "Built-in gain (loss): fair market value at contribution less the contributor's basis \
             (IRC §704(c)(1)(A)); a built-in loss is taken into account only for the contributing \
             member (IRC §704(c)(1)(C)).",
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
    /// In-service date, ISO 8601.
    pub in_service: String,
    /// Capitalized cost.
    pub cost: Decimal,
    /// Salvage value.
    pub salvage: Decimal,
    /// Useful life in months.
    pub life_months: u32,
    /// Accumulated depreciation by schedule (zero once disposed).
    pub accumulated: Decimal,
    /// Book value (zero once disposed).
    pub book_value: Decimal,
    /// "Active", or "Disposed 2026-09-30, proceeds 450.00".
    pub status: String,
    /// Disposed by the date (shown, not in the totals).
    pub disposed: bool,
    /// The tax basis rule, if recorded.
    pub tax_rule: Option<TaxBasisRule>,
    /// The contributing member's adjusted basis (a contribution only).
    pub contributor_basis: Option<Decimal>,
    /// The depreciable federal tax basis, if recorded.
    pub tax_basis: Option<Decimal>,
    /// Built-in gain (+) or loss (−) at contribution, if any.
    pub built_in_gain: Option<Decimal>,
}

/// Which register column a reconciliation amount sits under.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReconColumn {
    /// Under "Cost".
    Cost,
    /// Under "Accumulated depreciation".
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
    /// Total cost, accumulated depreciation, and book value of the assets
    /// held.
    pub totals: [Decimal; 3],
    /// Total tax basis of the assets held that have one.
    pub tax_total: Decimal,
    /// [`missing_line`] of the register's missing_tax_basis.
    pub missing: Option<String>,
    /// [`notes`].
    pub notes: Vec<Note>,
    /// Cost then accumulated depreciation: per register, per books,
    /// difference.
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
                in_service: a.in_service.to_string(),
                cost: a.cost,
                salvage: a.salvage,
                life_months: a.useful_life_months,
                accumulated: a.accumulated,
                book_value: a.book_value,
                status: status(a),
                disposed: a.disposed.is_some(),
                tax_rule: a.tax_rule,
                contributor_basis: a.contributor_basis,
                tax_basis: a.tax_basis,
                built_in_gain: a.built_in_gain,
            })
            .collect(),
        totals: [r.total_cost, r.total_accumulated, r.total_book_value],
        tax_total: r.total_tax_basis,
        missing: missing_line(r.missing_tax_basis),
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
                r.total_accumulated,
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
        ],
    }
}

/// The fixed asset register as XLSX bytes.
///
/// Layout as every statement (crate::xlsx INVARIANT 4): company, title,
/// and date on rows 1-3, headings ([`XLSX_COLUMNS`]) on row 5, one row
/// per asset from row 6, amounts as numbers in the accounting format, the
/// tax rule in words with its citation ([`rule_cited`]); then the totals
/// row (double rule) with the tax basis total under Tax basis, the
/// missing tax basis line when there is one, an empty row, "Reconciliation
/// to the books" and its lines with amounts under Cost and Accumulated
/// depreciation, an empty row, and "Notes" with [`notes`].
pub fn asset_register_xlsx(r: &AssetRegister, meta: &ReportMeta) -> Result<Vec<u8>, XlsxError> {
    const AMOUNTS: [u16; 7] = [4, 5, 7, 8, 10, 11, 12];
    const TOTALS: [u16; 4] = [4, 7, 8, 11];
    let l = layout(r);
    let mut workbook = Workbook::new();
    let sheet = workbook.add_worksheet();
    let f = Formats::new();
    sheet.set_name(TITLE)?;
    let width = XLSX_COLUMNS.len() as u16;
    let mut row = header(sheet, &f, meta, &l.title, &l.period, width)?;
    for (i, h) in XLSX_COLUMNS.iter().enumerate() {
        let fmt = if AMOUNTS.contains(&(i as u16)) || i == 6 {
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
        sheet.write_string_with_format(row, 3, &a.in_service, text)?;
        number(sheet, row, 4, a.cost, amount)?;
        number(sheet, row, 5, a.salvage, amount)?;
        sheet.write_number_with_format(
            row,
            6,
            f64::from(a.life_months),
            &text.clone().set_align(FormatAlign::Right),
        )?;
        number(sheet, row, 7, a.accumulated, amount)?;
        number(sheet, row, 8, a.book_value, amount)?;
        match a.tax_rule {
            Some(rule) => sheet.write_string_with_format(row, 9, rule_cited(rule), text)?,
            None => sheet.write_string_with_format(
                row,
                9,
                NOT_RECORDED,
                if a.disposed { text } else { &missing },
            )?,
        };
        for (c, v) in [
            (10, a.contributor_basis),
            (11, a.tax_basis),
            (12, a.built_in_gain),
        ] {
            if let Some(v) = v {
                number(sheet, row, c, v, amount)?;
            }
        }
        sheet.write_string_with_format(row, 13, &a.status, text)?;
        row += 1;
    }
    sheet.write_string_with_format(row, 0, "Total, assets held", &f.bold)?;
    for c in (1..width).filter(|c| !TOTALS.contains(c)) {
        sheet.write_blank(row, c, &f.dollars_total)?;
    }
    for (c, d) in TOTALS
        .into_iter()
        .zip(l.totals.into_iter().chain([l.tax_total]))
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
            ReconColumn::Cost => 4,
            ReconColumn::Accumulated => 7,
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
        (4, 14.0),
        (5, 12.0),
        (6, 9.0),
        (7, 16.0),
        (8, 14.0),
        (9, 30.0),
        (10, 14.0),
        (11, 14.0),
        (12, 14.0),
        (13, 34.0),
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
        "lines": l.lines.iter().map(|a| serde_json::json!({
            "tag": a.tag,
            "description": a.description,
            "serial": a.serial,
            "in_service": a.in_service,
            "cost": amount(a.cost),
            "salvage": amount(a.salvage),
            "life": a.life_months.to_string(),
            "accumulated": amount(a.accumulated),
            "book_value": amount(a.book_value),
            "status": a.status,
            "disposed": a.disposed,
            "rule": a.tax_rule.map_or(NOT_RECORDED, rule_label),
            "rule_mark": a.tax_rule.map_or("", rule_mark),
            "recorded": a.tax_rule.is_some(),
            "contributor_basis": maybe(a.contributor_basis),
            "tax_basis": maybe(a.tax_basis),
            "built_in_gain": maybe(a.built_in_gain),
        })).collect::<Vec<_>>(),
        "totals": l.totals.iter().map(|d| amount(*d)).collect::<Vec<_>>(),
        "tax_total": amount(l.tax_total),
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
