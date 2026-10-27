//! Reading the words and pages out of the files the books keep: a PDF's
//! text and a page as a picture, an XLSX's cells as exact decimal strings,
//! and plain text as it is (plan P6 step 1, D-32, D-53).
//!
//! ```text
//! GOAL: one extractor, in this process, that the server's filing scan,
//!       the journal's extraction record and the viewer all read a file
//!       through, so a receipt's text never depends on which door it came
//!       in; a reader failure is an error, never a panic out of the
//!       server.
//! SOURCE: plan/served.md P6 step 1 (D-32: no sandbox; D-53: in-process,
//!         no new crate); IRS Pub. 583 (records must be readable); D-15
//!         (no OCR: an image-only receipt is its page pictures).
//! INVARIANTS:
//!   1. Deterministic: the same bytes give the same text and the same PNG
//!      bytes, every run.
//!   2. Text is read, never guessed: an XLSX cell's number is the decimal
//!      string the sheet XML holds, never a binary float; a PDF's text is
//!      its glyphs' ToUnicode text, page by page, line by line.
//!   3. Caps: at most 25 MiB of input, 30 pages, 2 MiB of text. Past the
//!      input cap nothing is read; past the page cap the rest is not
//!      read; past the text cap the text is cut at a character boundary.
//!   4. A panic inside the PDF reader is caught here and reported as
//!      [`ExtractError::ReaderPanicked`]: the process is not isolated
//!      (D-32, and the plan says so).
//! ```

use std::io::{Cursor, Read};
use thiserror::Error;

/// The largest file extraction reads: 25 MiB (plan P6 step 1).
pub const MAX_BYTES: usize = 25 * 1024 * 1024;

/// The most pages of a PDF extraction reads.
pub const MAX_PAGES: usize = 30;

/// The most text one extraction keeps: 2 MiB.
pub const MAX_TEXT_BYTES: usize = 2 * 1024 * 1024;

/// What read a file's text, as the extraction record names it.
pub const EXTRACTOR: &str = "wanderbooks";

/// That extractor's version: a new one journals a new extraction record,
/// never replaces an old one (plan P6 step 2).
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// What reading a file found (plan P6 step 2's `status`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExtractStatus {
    /// Text was read.
    Ok,
    /// There is no text to read (an image is its page pictures, D-15).
    None,
    /// Reading was tried and failed.
    Failed,
}

impl ExtractStatus {
    /// Its wire name, as the extraction record journals it.
    pub fn key(self) -> &'static str {
        match self {
            ExtractStatus::Ok => "ok",
            ExtractStatus::None => "none",
            ExtractStatus::Failed => "failed",
        }
    }
}

/// What [`extract`] found: the text (when any), the file's page count, and
/// whether there was text to read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Extracted {
    /// The text, capped at [`MAX_TEXT_BYTES`].
    pub text: Option<String>,
    /// How many pages the file has (0 when it has none).
    pub pages: u32,
    /// What was found.
    pub status: ExtractStatus,
}

/// Why a file's text or page could not be read.
#[derive(Debug, Error)]
pub enum ExtractError {
    /// The file is past [`MAX_BYTES`].
    #[error("the file is {size} bytes: extraction reads at most {MAX_BYTES} bytes")]
    TooLarge {
        /// The file's size.
        size: usize,
    },
    /// The bytes are not a PDF this reader can read.
    #[error("not a readable PDF: {0}")]
    Pdf(String),
    /// The bytes are not a spreadsheet this reader can read.
    #[error("not a readable spreadsheet: {0}")]
    Xlsx(String),
    /// The page asked for is past the document's last.
    #[error("page {page} is past the document's {pages} page(s)")]
    PageOutOfRange {
        /// The page asked for (1-based).
        page: usize,
        /// How many pages the document has.
        pages: usize,
    },
    /// The page size asked for is zero.
    #[error("a page image needs a size of at least 1 pixel")]
    NoPixels,
    /// The PDF reader panicked on the file (INVARIANT 4).
    #[error("the PDF reader stopped on that file")]
    ReaderPanicked,
}

/// Run `read`, turning a panic into [`ExtractError::ReaderPanicked`]
/// (INVARIANT 4).
fn caught<T>(read: impl FnOnce() -> Result<T, ExtractError>) -> Result<T, ExtractError> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(read))
        .unwrap_or(Err(ExtractError::ReaderPanicked))
}

/// The text a PDF draws: each glyph's text, left to right, one `String`
/// per glyph, one `Vec` per line, from the top of each page down, page
/// after page (INVARIANT 2). At most [`MAX_PAGES`] pages are read.
pub fn pdf_lines(bytes: &[u8]) -> Result<Vec<Vec<String>>, ExtractError> {
    if bytes.len() > MAX_BYTES {
        return Err(ExtractError::TooLarge { size: bytes.len() });
    }
    caught(|| read_pdf_lines(bytes))
}

/// The text a PDF draws, its glyphs joined into lines and its pages into
/// one text (INVARIANTS 1 and 2), capped at [`MAX_TEXT_BYTES`].
pub fn pdf_text(bytes: &[u8]) -> Result<String, ExtractError> {
    let lines = pdf_lines(bytes)?;
    let text = lines
        .iter()
        .map(|l| l.concat())
        .collect::<Vec<_>>()
        .join("\n");
    Ok(cap_text(text))
}

/// How many pages a PDF has.
pub fn page_count(bytes: &[u8]) -> Result<usize, ExtractError> {
    if bytes.len() > MAX_BYTES {
        return Err(ExtractError::TooLarge { size: bytes.len() });
    }
    caught(|| Ok(read_pdf(bytes)?.pages().len()))
}

/// One page of a PDF as a PNG whose longest side is at most `max_side_px`
/// pixels (`page` is 0-based; `max_side_px` of 0 is refused). The same
/// bytes, page and size give the same PNG bytes (INVARIANT 1).
pub fn page_png(bytes: &[u8], page: usize, max_side_px: u32) -> Result<Vec<u8>, ExtractError> {
    if bytes.len() > MAX_BYTES {
        return Err(ExtractError::TooLarge { size: bytes.len() });
    }
    if max_side_px == 0 {
        return Err(ExtractError::NoPixels);
    }
    caught(|| read_page_png(bytes, page, max_side_px))
}

/// The bytes as text, if they are text the books read (INVARIANT 2): a
/// PDF's text, an XLSX's cells, UTF-16 with its byte-order mark, UTF-8, or,
/// for a text type or name, whatever they decode to. Text with NUL bytes
/// and no byte-order mark (UTF-16 written without one is valid UTF-8, its
/// digits split by NULs) is read every way it can be (as it is, as UTF-16
/// both ways round, and with its NULs taken out), each reading on a line of
/// its own: so is bytes that aren't UTF-8 but are NUL-interleaved as UTF-16
/// is. At most [`MAX_TEXT_BYTES`] is returned.
pub fn text_of(bytes: &[u8], mime_type: &str, file_name: &str) -> Option<String> {
    extract(bytes, mime_type, file_name).text
}

/// Read a file the way the books record it (plan P6 step 2): a PDF's text
/// and pages, an XLSX's cells, plain text as it is; an image has none. A
/// reader failure is [`ExtractStatus::Failed`], never a panic (INVARIANT 4).
pub fn extract(bytes: &[u8], mime_type: &str, file_name: &str) -> Extracted {
    if bytes.len() > MAX_BYTES {
        return Extracted {
            text: None,
            pages: 0,
            status: ExtractStatus::Failed,
        };
    }
    let mime = mime_type.trim().to_ascii_lowercase();
    let ext = std::path::Path::new(file_name)
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .unwrap_or_default();
    let magic_pdf = bytes.starts_with(b"%PDF");
    if mime == "application/pdf" || magic_pdf {
        return match pdf_text(bytes) {
            Ok(text) => {
                let pages = page_count(bytes).unwrap_or(0).min(MAX_PAGES) as u32;
                found(Some(text), pages)
            }
            // A file CALLED a PDF whose bytes are text (a text file named
            // .pdf): read the text, so the TIN scan and the reader see what
            // it says (B-26: the scan fails closed). One with the PDF magic
            // that won't read is a failed extraction.
            Err(_) if !magic_pdf => match plain_text(bytes, &mime, &ext) {
                Some(text) => found(Some(text), 1),
                None => Extracted {
                    text: None,
                    pages: 0,
                    status: ExtractStatus::Failed,
                },
            },
            Err(_) => Extracted {
                text: None,
                pages: 0,
                status: ExtractStatus::Failed,
            },
        };
    }
    let xlsx = "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet";
    if mime == xlsx || ext == "xlsx" || ext == "xlsm" {
        return match xlsx_text(bytes) {
            Ok(text) => found(Some(text), 1),
            Err(_) => Extracted {
                text: None,
                pages: 0,
                status: ExtractStatus::Failed,
            },
        };
    }
    match plain_text(bytes, &mime, &ext) {
        Some(text) => found(Some(text), 1),
        None => Extracted {
            text: None,
            pages: 0,
            status: ExtractStatus::None,
        },
    }
}

/// An extraction's answer from the text read: `none` when it is blank.
fn found(text: Option<String>, pages: u32) -> Extracted {
    match text {
        Some(t) if t.trim().is_empty() => Extracted {
            text: None,
            pages,
            status: ExtractStatus::None,
        },
        Some(t) => Extracted {
            text: Some(t),
            pages,
            status: ExtractStatus::Ok,
        },
        None => Extracted {
            text: None,
            pages,
            status: ExtractStatus::None,
        },
    }
}

/// The plain-text reading of bytes (the second half of [`text_of`]).
fn plain_text(bytes: &[u8], mime: &str, ext: &str) -> Option<String> {
    let utf16 = |body: &[u8], le: bool| {
        let units: Vec<u16> = body
            .as_chunks::<2>()
            .0
            .iter()
            .map(|&c| {
                if le {
                    u16::from_le_bytes(c)
                } else {
                    u16::from_be_bytes(c)
                }
            })
            .collect();
        String::from_utf16_lossy(&units)
    };
    let text = match bytes {
        [0xFF, 0xFE, body @ ..] => utf16(body, true),
        [0xFE, 0xFF, body @ ..] => utf16(body, false),
        _ => {
            let plain = match std::str::from_utf8(bytes) {
                Ok(t) => Some(t.to_string()),
                Err(_) if text_like(mime, ext) => Some(String::from_utf8_lossy(bytes).into_owned()),
                Err(_) => None,
            };
            if !bytes.contains(&0) || plain.is_none() && !nul_interleaved(bytes) {
                return plain.map(cap_text);
            }
            let without_nuls: Vec<u8> = bytes.iter().copied().filter(|&b| b != 0).collect();
            let readings = [
                plain.unwrap_or_default(),
                utf16(bytes, true),
                utf16(bytes, false),
                String::from_utf8_lossy(&without_nuls).into_owned(),
            ];
            readings.join("\n")
        }
    };
    Some(cap_text(text))
}

/// [`text_of`]'s answer capped at [`MAX_TEXT_BYTES`], cut at a character
/// boundary.
fn cap_text(mut text: String) -> String {
    if text.len() > MAX_TEXT_BYTES {
        let mut cut = MAX_TEXT_BYTES;
        while !text.is_char_boundary(cut) {
            cut -= 1;
        }
        text.truncate(cut);
    }
    text
}

/// Whether a file of this type or name is text: csv, tsv, txt, json, iif,
/// xml, html, md and their kin.
fn text_like(mime: &str, ext: &str) -> bool {
    mime.starts_with("text/")
        || [
            "json",
            "xml",
            "csv",
            "x-iif",
            "javascript",
            "x-yaml",
            "yaml",
        ]
        .iter()
        .any(|t| mime.ends_with(&format!("/{t}")) || mime.ends_with(&format!("+{t}")))
        || [
            "csv", "tsv", "txt", "text", "json", "iif", "xml", "html", "htm", "md", "markdown",
            "yaml", "yml", "tab", "prn", "log", "ofx", "qfx", "qif",
        ]
        .contains(&ext)
}

/// Whether `bytes` look like UTF-16 without a byte-order mark: of their
/// byte pairs, at least half have a NUL on one side (the high byte of a
/// Latin character) and at most an eighth on the other.
#[doc(hidden)]
pub fn nul_interleaved(bytes: &[u8]) -> bool {
    let pairs = bytes.as_chunks::<2>().0;
    let nuls = |side: usize| pairs.iter().filter(|p| p[side] == 0).count();
    let (first, second) = (nuls(0), nuls(1));
    let n = pairs.len();
    n > 0 && (2 * first >= n && 8 * second <= n || 2 * second >= n && 8 * first <= n)
}

// ---------------------------------------------------------------------------
// PDF
// ---------------------------------------------------------------------------

type Pdf = hayro_interpret::hayro_syntax::Pdf;

/// [`Pdf::new`] with its error as ours.
fn read_pdf(bytes: &[u8]) -> Result<Pdf, ExtractError> {
    Pdf::new(bytes.to_vec()).map_err(|e| ExtractError::Pdf(format!("{e:?}")))
}

fn read_pdf_lines(bytes: &[u8]) -> Result<Vec<Vec<String>>, ExtractError> {
    use hayro_interpret::font::GlyphRun;
    use hayro_interpret::hayro_cmap::BfString;
    use hayro_interpret::{
        BlendMode, ClipPath, Context, Device, DrawMode, DrawProps, Image, ImageDrawProps,
        InterpreterCache, InterpreterSettings, SoftMask, interpret_page,
    };
    use kurbo::{Affine, BezPath, Point, Rect};
    /// Each glyph drawn: its baseline (hundredths of a point), x, and text.
    struct Glyphs(Vec<(i64, f64, String)>);
    impl<'a> Device<'a> for Glyphs {
        fn draw_path(&mut self, _: &BezPath, _: DrawProps<'a>, _: &DrawMode) {}
        fn push_clip_path(&mut self, _: &ClipPath) {}
        fn push_transparency_group(&mut self, _: f32, _: Option<SoftMask<'a>>, _: BlendMode) {}
        fn draw_glyph_run(&mut self, run: &GlyphRun<'_, 'a>, props: DrawProps<'a>, _: &DrawMode) {
            for g in run.glyphs() {
                let at = (props.transform * g.transform()) * Point::ZERO;
                let text = match g.as_unicode() {
                    Some(BfString::Char(c)) => c.to_string(),
                    Some(BfString::String(s)) => s,
                    // A mark or a ligature's second glyph: its text is with
                    // the cluster's first.
                    None => String::new(),
                };
                self.0.push(((at.y * 100.0).round() as i64, at.x, text));
            }
        }
        fn draw_image(&mut self, _: Image<'a, '_>, _: ImageDrawProps<'a>) {}
        fn pop_clip(&mut self) {}
        fn pop_transparency_group(&mut self) {}
    }
    let pdf = read_pdf(bytes)?;
    let cache = InterpreterCache::new();
    let mut out = Vec::new();
    for page in pdf.pages().iter().take(MAX_PAGES) {
        let mut ctx = Context::new(
            Affine::IDENTITY,
            Rect::new(0.0, 0.0, 1.0, 1.0),
            &cache,
            pdf.xref(),
            InterpreterSettings::default(),
        );
        let mut glyphs = Glyphs(Vec::new());
        interpret_page(page, &mut ctx, &mut glyphs);
        // PDF space runs up the page: the highest baseline first.
        let mut lines: std::collections::BTreeMap<std::cmp::Reverse<i64>, Vec<(f64, String)>> =
            std::collections::BTreeMap::new();
        for (y, x, t) in glyphs.0 {
            lines.entry(std::cmp::Reverse(y)).or_default().push((x, t));
        }
        for (_, mut line) in lines {
            line.sort_by(|a, b| a.0.total_cmp(&b.0));
            out.push(line.into_iter().map(|(_, t)| t).collect());
        }
    }
    Ok(out)
}

fn read_page_png(bytes: &[u8], page: usize, max_side_px: u32) -> Result<Vec<u8>, ExtractError> {
    use hayro::hayro_interpret::InterpreterSettings;
    use hayro::vello_cpu::color::palette::css::WHITE;
    use hayro::{PixmapSettings, RenderCache, RenderSettings, render};
    let pdf = read_pdf(bytes)?;
    let pages = pdf.pages();
    let found = pages.get(page).ok_or(ExtractError::PageOutOfRange {
        page: page + 1,
        pages: pages.len(),
    })?;
    let (w, h) = found.render_dimensions();
    // The page's longest side at most `max_side_px`; never zero.
    let scale = max_side_px as f32 / w.max(h).max(1.0);
    let scale = scale.max(1.0 / w.max(h).max(1.0));
    let px = PixmapSettings {
        x_scale: scale,
        y_scale: scale,
        bg_color: WHITE,
    };
    let pixmap = render(
        found,
        &RenderCache::new(),
        &InterpreterSettings::default(),
        &RenderSettings::default(),
        &px,
    );
    pixmap
        .into_png()
        .map_err(|e| ExtractError::Pdf(format!("{e:?}")))
}

// ---------------------------------------------------------------------------
// XLSX
// ---------------------------------------------------------------------------

/// An XLSX's cells as text: one line per row, one cell per column,
/// separated by tabs; a number is the exact decimal string the sheet XML
/// holds, never a binary float (INVARIANT 2).
pub fn xlsx_text(bytes: &[u8]) -> Result<String, ExtractError> {
    if bytes.len() > MAX_BYTES {
        return Err(ExtractError::TooLarge { size: bytes.len() });
    }
    let mut zip =
        zip::ZipArchive::new(Cursor::new(bytes)).map_err(|e| ExtractError::Xlsx(e.to_string()))?;
    // The workbook's shared strings, in index order.
    let shared = match read_zip(&mut zip, "xl/sharedStrings.xml") {
        Some(xml) => shared_strings(&xml)?,
        None => Vec::new(),
    };
    let names: Vec<String> = zip
        .file_names()
        .filter_map(|n| n.ok().map(|n| n.into_owned()))
        .filter(|n| n.starts_with("xl/worksheets/sheet") && n.ends_with(".xml"))
        .collect();
    let mut sheets = names;
    sheets.sort();
    let mut out = String::new();
    for name in sheets {
        let Some(xml) = read_zip(&mut zip, &name) else {
            continue;
        };
        let text = sheet_cells(&xml, &shared)?;
        if !text.is_empty() {
            if !out.is_empty() {
                out.push('\n');
            }
            out.push_str(&text);
        }
    }
    Ok(cap_text(out))
}

/// A zip member's text, if it is there.
fn read_zip(zip: &mut zip::ZipArchive<Cursor<&[u8]>>, name: &str) -> Option<String> {
    let mut file = zip.by_name(name).ok()?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes).ok()?;
    Some(String::from_utf8_lossy(&bytes).into_owned())
}

/// The text of each `<si>` in `xl/sharedStrings.xml`, rich-text runs
/// concatenated.
fn shared_strings(xml: &str) -> Result<Vec<String>, ExtractError> {
    let mut reader = quick_xml::Reader::from_str(xml);
    let mut out = Vec::new();
    let mut current: Option<String> = None;
    let mut in_text = false;
    loop {
        match reader.read_event() {
            Ok(quick_xml::events::Event::Start(e)) if e.name().as_ref() == b"si" => {
                current = Some(String::new());
            }
            Ok(quick_xml::events::Event::End(e)) if e.name().as_ref() == b"si" => {
                out.push(current.take().unwrap_or_default());
            }
            Ok(quick_xml::events::Event::Start(e)) if e.name().as_ref() == b"t" => in_text = true,
            Ok(quick_xml::events::Event::End(e)) if e.name().as_ref() == b"t" => in_text = false,
            Ok(quick_xml::events::Event::Text(t)) if in_text => {
                if let Some(s) = &mut current {
                    s.push_str(&t.decode().map_err(|e| ExtractError::Xlsx(e.to_string()))?);
                }
            }
            Ok(quick_xml::events::Event::Eof) => break,
            Ok(_) => {}
            Err(e) => return Err(ExtractError::Xlsx(e.to_string())),
        }
    }
    Ok(out)
}

/// One worksheet's cells as tab-separated rows.
fn sheet_cells(xml: &str, shared: &[String]) -> Result<String, ExtractError> {
    use quick_xml::events::Event;
    let mut reader = quick_xml::Reader::from_str(xml);
    let mut rows: Vec<Vec<String>> = Vec::new();
    let mut row: Vec<String> = Vec::new();
    let mut cell_type = String::new();
    let mut in_value = false;
    let mut value = String::new();
    let mut in_inline = false;
    let mut inline = String::new();
    let mut in_inline_text = false;
    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) => match e.name().as_ref() {
                b"row" => row = Vec::new(),
                b"c" => {
                    cell_type.clear();
                    value.clear();
                    inline.clear();
                    for a in e.attributes().flatten() {
                        if a.key.as_ref() == b"t" {
                            cell_type = String::from_utf8_lossy(&a.value).into_owned();
                        }
                    }
                }
                b"v" => {
                    in_value = true;
                    value.clear();
                }
                b"is" => in_inline = true,
                b"t" if in_inline => in_inline_text = true,
                _ => {}
            },
            Ok(Event::End(e)) => match e.name().as_ref() {
                b"row" => rows.push(std::mem::take(&mut row)),
                b"c" => {
                    let text = match cell_type.as_str() {
                        "s" => value
                            .parse::<usize>()
                            .ok()
                            .and_then(|i| shared.get(i).cloned())
                            .unwrap_or_default(),
                        "inlineStr" => inline.clone(),
                        "b" => match value.as_str() {
                            "1" => "TRUE".into(),
                            _ => "FALSE".into(),
                        },
                        _ => value.clone(),
                    };
                    row.push(text);
                }
                b"v" => in_value = false,
                b"is" => in_inline = false,
                b"t" if in_inline => in_inline_text = false,
                _ => {}
            },
            Ok(Event::Text(t)) => {
                let text = t.decode().map_err(|e| ExtractError::Xlsx(e.to_string()))?;
                if in_inline_text {
                    inline.push_str(&text);
                } else if in_value {
                    value.push_str(&text);
                }
            }
            Ok(Event::Eof) => break,
            Ok(_) => {}
            Err(e) => return Err(ExtractError::Xlsx(e.to_string())),
        }
    }
    Ok(rows
        .iter()
        .map(|r| r.join("\t"))
        .collect::<Vec<_>>()
        .join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// INVARIANT 4: a panic inside the reader is caught, never fatal.
    #[test]
    fn a_reader_panic_is_reported_not_raised() {
        let got = caught::<()>(|| panic!("the reader broke"));
        assert!(matches!(got, Err(ExtractError::ReaderPanicked)));
    }

    /// INVARIANT 3: a file past the input cap is not read.
    #[test]
    fn too_large_is_refused_without_reading() {
        let bytes = vec![0u8; MAX_BYTES + 1];
        assert!(matches!(
            pdf_text(&bytes),
            Err(ExtractError::TooLarge { .. })
        ));
        assert!(text_of(&bytes, "text/plain", "big.txt").is_none());
    }
}
