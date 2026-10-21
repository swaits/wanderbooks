//! Printable asset label sheets: one tag per label, with a QR code.
//!
//! Each label carries the company name, the asset tag (its headline), what
//! the asset is, its serial number when recorded, and a QR code holding
//! exactly the tag. Labels are typeset by `templates/labels.typ` with the
//! same bundled fonts as [`crate::pdf`]; the labels reach Typst as
//! JSON data (`sys.inputs.data`), never as markup. The sheet geometry comes
//! from the label maker's own specification, vendored in
//! `docs/spec/sources/avery-5160.md`.
//!
//! ```text
//! GOAL: tag every fixed asset with a label a phone can scan back to its
//!       record, printed on stock label sheets.
//! SOURCE: docs/spec/sources/avery-5160.md (Avery 5160: 1" x 2-5/8", 30 per
//!         sheet, 3 x 10, top margin 0.5", side margin 0.1875", column pitch
//!         2.75", row pitch 1"); ISO/IEC 18004 QR codes via the `qrcode`
//!         crate, error correction level M; CONVENTION (what a label shows).
//! INVARIANTS:
//!   1. Every label prints inside its cell: each label is a clipped box the
//!      size of its cell, and its text shrinks, then ellipsizes, to fit (the
//!      description in two lines, the rest in one; the tag shrinks to 6 pt
//!      before it would ever be cut, and the QR code always holds it whole).
//!   2. Order: labels fill positions left to right, then top to bottom,
//!      starting at position `skip` of the first sheet, then on to the
//!      next sheet at position 0.
//!   3. One page per sheet: pages = ceil((skip + labels) / per_sheet).
//!   4. The QR code decodes to exactly the tag.
//!   5. Deterministic: the same labels, company, sheet, and skip render to
//!      the same bytes.
//!   6. Text is data: a description holding Typst syntax ("#panic()",
//!      "*bold*") prints literally and never runs.
//!   7. Refused: no labels; skip >= per_sheet.
//! SCENARIOS:
//!   - 1 label, skip 0: one page, the label at the top-left cell.
//!   - 30 labels, skip 0: one full sheet. 30 labels, skip 1: two pages, the
//!     last label at the top-left of sheet 2.
//!   - 61 labels: three pages, one label on the third.
//!   - A partly used sheet with 7 labels left: skip 23.
//! ```

use crate::pdf::PdfError;
use qrcode::render::svg;
use qrcode::{EcLevel, QrCode};
use std::sync::OnceLock;
use typst::foundations::{Dict, IntoValue};
use typst_as_lib::{TypstEngine, TypstTemplateMainFile};
use typst_layout::PagedDocument;

/// One asset label.
#[derive(Clone, Debug)]
pub struct AssetLabel {
    /// The asset tag, e.g. "FL-0001" (the label's headline and the QR code's content).
    pub tag: String,
    /// What it is, e.g. "Framework Laptop 13".
    pub description: String,
    /// Manufacturer serial number, if recorded.
    pub serial: Option<String>,
}

/// A sheet layout.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LabelSheet {
    /// Avery 5160 (and compatibles): 1" x 2-5/8", 30 per US Letter sheet.
    Avery5160,
}

/// Where a sheet's labels sit, in PDF points (1/72 inch), measured from the
/// page's top-left corner.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SheetGeometry {
    /// Page width.
    pub page_width: f64,
    /// Page height.
    pub page_height: f64,
    /// One label's width.
    pub label_width: f64,
    /// One label's height.
    pub label_height: f64,
    /// Left edge of the first column (the side margin).
    pub left: f64,
    /// Top edge of the first row (the top margin).
    pub top: f64,
    /// Distance from one column's left edge to the next.
    pub pitch_x: f64,
    /// Distance from one row's top edge to the next.
    pub pitch_y: f64,
    /// Labels across.
    pub columns: usize,
    /// Labels down.
    pub rows: usize,
    /// Space kept clear inside each label's edge (printer drift).
    pub inset: f64,
}

/// A label's cell on its page: `(x, y, width, height)` in points from the
/// page's top-left corner.
pub type Cell = (f64, f64, f64, f64);

impl SheetGeometry {
    /// Labels per sheet.
    pub fn per_sheet(&self) -> usize {
        self.columns * self.rows
    }

    /// The cell at `position` (0-based, reading order) on a sheet.
    pub fn cell(&self, position: usize) -> Cell {
        let (row, col) = (position / self.columns, position % self.columns);
        (
            self.left + col as f64 * self.pitch_x,
            self.top + row as f64 * self.pitch_y,
            self.label_width,
            self.label_height,
        )
    }
}

const INCH: f64 = 72.0;

impl LabelSheet {
    /// The sheet's geometry, from the maker's specification
    /// (docs/spec/sources/avery-5160.md).
    pub fn geometry(self) -> SheetGeometry {
        match self {
            LabelSheet::Avery5160 => SheetGeometry {
                page_width: 8.5 * INCH,
                page_height: 11.0 * INCH,
                label_width: 2.625 * INCH,
                label_height: 1.0 * INCH,
                left: 0.1875 * INCH,
                top: 0.5 * INCH,
                pitch_x: 2.75 * INCH,
                pitch_y: 1.0 * INCH,
                columns: 3,
                rows: 10,
                inset: 4.0,
            },
        }
    }
}

static TEMPLATE: &str = include_str!("../templates/labels.typ");

fn engine() -> &'static TypstEngine<TypstTemplateMainFile> {
    static ENGINE: OnceLock<TypstEngine<TypstTemplateMainFile>> = OnceLock::new();
    ENGINE.get_or_init(|| {
        TypstEngine::builder()
            .main_file(TEMPLATE)
            .fonts(crate::fonts::typst_fonts())
            .build()
    })
}

/// The QR code for `tag` (error correction level M) as an SVG document,
/// its 4-module quiet zone included.
pub fn qr_svg(tag: &str) -> Result<String, PdfError> {
    let code = QrCode::with_error_correction_level(tag.as_bytes(), EcLevel::M)
        .map_err(|e| PdfError::Qr(format!("{tag:?}: {e}")))?;
    Ok(code
        .render::<svg::Color<'_>>()
        .quiet_zone(true)
        .module_dimensions(1, 1)
        .dark_color(svg::Color("#000000"))
        .light_color(svg::Color("#ffffff"))
        .build())
}

/// The template's input: the geometry and every sheet's positions, with
/// each label's QR code rendered.
pub fn template_data(
    labels: &[AssetLabel],
    company: &str,
    sheet: LabelSheet,
    skip: usize,
) -> Result<serde_json::Value, PdfError> {
    let g = sheet.geometry();
    let per = g.per_sheet();
    if labels.is_empty() {
        return Err(PdfError::NoLabels);
    }
    if skip >= per {
        return Err(PdfError::SkipOutOfRange {
            skip,
            per_sheet: per,
        });
    }
    let mut positions: Vec<serde_json::Value> = vec![serde_json::Value::Null; skip];
    for l in labels {
        positions.push(serde_json::json!({
            "tag": l.tag,
            "description": l.description,
            "serial": l.serial,
            "qr": qr_svg(&l.tag)?,
        }));
    }
    let pages: Vec<_> = positions.chunks(per).map(<[_]>::to_vec).collect();
    Ok(serde_json::json!({
        "company": company,
        "geometry": {
            "page_width": g.page_width,
            "page_height": g.page_height,
            "label_width": g.label_width,
            "label_height": g.label_height,
            "left": g.left,
            "top": g.top,
            "pitch_x": g.pitch_x,
            "pitch_y": g.pitch_y,
            "columns": g.columns,
            "rows": g.rows,
            "inset": g.inset,
        },
        "pages": pages,
    }))
}

/// Typeset label sheets into a document (pages, before PDF encoding), so
/// the layout can be inspected. Same arguments and refusals as
/// [`asset_labels_pdf`].
pub fn typeset_labels(
    labels: &[AssetLabel],
    company: &str,
    sheet: LabelSheet,
    skip: usize,
) -> Result<PagedDocument, PdfError> {
    let data = template_data(labels, company, sheet, skip)?;
    let mut inputs = Dict::new();
    inputs.insert("data".into(), data.to_string().into_value());
    engine()
        .compile_with_input(inputs)
        .output
        .map_err(|e| PdfError::Compile(format!("{e:?}")))
}

/// Render labels onto sheets, starting `skip` positions into the first sheet (to reuse a partly used sheet), as PDF bytes.
///
/// See the module documentation for the spec. Refuses an empty `labels`
/// ([`PdfError::NoLabels`]) and `skip` of a whole sheet or more
/// ([`PdfError::SkipOutOfRange`]).
pub fn asset_labels_pdf(
    labels: &[AssetLabel],
    company: &str,
    sheet: LabelSheet,
    skip: usize,
) -> Result<Vec<u8>, PdfError> {
    let doc = typeset_labels(labels, company, sheet, skip)?;
    typst_pdf::pdf(&doc, &typst_pdf::PdfOptions::default())
        .map_err(|e| PdfError::Save(format!("{e:?}")))
}
