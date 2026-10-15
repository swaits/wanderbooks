//! Spec tests for `ledger_import::labels` (asset label sheets). Each test
//! names the module INVARIANT it checks.

use ledger_import::labels::{SheetGeometry, qr_svg, template_data, typeset_labels};
use ledger_import::{AssetLabel, LabelSheet, PdfError, asset_labels_pdf};
use typst::layout::{Abs, Frame, FrameItem, Point, Transform};

const COMPANY: &str = "Fernhill Labs LLC";

fn label(i: usize) -> AssetLabel {
    AssetLabel {
        tag: format!("FL-{i:04}"),
        description: format!("Asset number {i}"),
        serial: i.is_multiple_of(2).then(|| format!("SN{i:08}")),
    }
}

fn labels(n: usize) -> Vec<AssetLabel> {
    (1..=n).map(label).collect()
}

fn pages(n: usize, skip: usize) -> usize {
    typeset_labels(&labels(n), COMPANY, LabelSheet::Avery5160, skip)
        .expect("typesets")
        .pages()
        .len()
}

/// SOURCE: docs/spec/sources/avery-5160.md, figure by figure, in inches.
#[test]
fn avery_5160_geometry_matches_the_vendored_spec() {
    let g = LabelSheet::Avery5160.geometry();
    let inch = |x: f64| x * 72.0;
    assert_eq!(g.page_width, inch(8.5));
    assert_eq!(g.page_height, inch(11.0));
    assert_eq!(g.label_width, inch(2.625));
    assert_eq!(g.label_height, inch(1.0));
    assert_eq!(g.top, inch(0.5));
    assert_eq!(g.left, inch(0.1875));
    assert_eq!(g.pitch_x, inch(2.75));
    assert_eq!(g.pitch_y, inch(1.0));
    assert_eq!((g.columns, g.rows, g.per_sheet()), (3, 10, 30));
    // The template's cell edges, in PDF points from the bottom-left corner.
    let lefts: Vec<f64> = (0..3).map(|c| g.cell(c).0).collect();
    assert_eq!(lefts, [13.5, 211.5, 409.5]);
    let bottoms: Vec<f64> = (0..10)
        .map(|r| g.page_height - (g.cell(r * 3).1 + g.label_height))
        .collect();
    assert_eq!(
        bottoms,
        [
            684.0, 612.0, 540.0, 468.0, 396.0, 324.0, 252.0, 180.0, 108.0, 36.0
        ]
    );
    // Margins and gaps close the page exactly.
    let gap = g.pitch_x - g.label_width;
    assert_eq!(gap, inch(0.125));
    assert_eq!(2.0 * g.left + 3.0 * g.label_width + 2.0 * gap, g.page_width);
    assert_eq!(2.0 * g.top + 10.0 * g.pitch_y, g.page_height);
}

/// INVARIANT 3: pages = ceil((skip + labels) / 30).
#[test]
fn one_page_per_sheet() {
    assert_eq!(pages(1, 0), 1);
    assert_eq!(pages(30, 0), 1);
    assert_eq!(pages(30, 1), 2);
    assert_eq!(pages(61, 0), 3);
    assert_eq!(pages(7, 23), 1);
    assert_eq!(pages(8, 23), 2);
}

/// INVARIANT 7: a skip of a whole sheet or more, and no labels, are refused.
#[test]
fn refusals() {
    let s = LabelSheet::Avery5160;
    for skip in [30, 31, usize::MAX] {
        let e = asset_labels_pdf(&labels(1), COMPANY, s, skip).unwrap_err();
        assert!(
            matches!(e, PdfError::SkipOutOfRange { skip: k, per_sheet: 30 } if k == skip),
            "{e:?}"
        );
        assert!(e.to_string().contains("0 to 29"), "{e}");
    }
    let e = asset_labels_pdf(&[], COMPANY, s, 0).unwrap_err();
    assert!(matches!(e, PdfError::NoLabels), "{e:?}");
    assert!(asset_labels_pdf(&labels(1), COMPANY, s, 29).is_ok());
}

/// INVARIANT 5: the same input renders to the same bytes.
#[test]
fn deterministic_bytes() {
    let ls = labels(5);
    let a = asset_labels_pdf(&ls, COMPANY, LabelSheet::Avery5160, 3).unwrap();
    let b = asset_labels_pdf(&ls, COMPANY, LabelSheet::Avery5160, 3).unwrap();
    assert!(a.starts_with(b"%PDF"));
    assert_eq!(a, b);
}

/// A laid-out item with its absolute bounding box on the page, in points
/// from the top-left corner.
#[derive(Debug)]
struct Placed {
    text: Option<String>,
    image: bool,
    x0: f64,
    y0: f64,
    x1: f64,
    y1: f64,
}

fn walk(frame: &Frame, ts: Transform, out: &mut Vec<Placed>) {
    for (pos, item) in frame.items() {
        let at = |dx: Abs, dy: Abs| Point::new(pos.x + dx, pos.y + dy).transform(ts);
        match item {
            FrameItem::Group(g) => {
                let ts = ts
                    .pre_concat(Transform::translate(pos.x, pos.y))
                    .pre_concat(g.transform);
                walk(&g.frame, ts, out);
            }
            FrameItem::Text(t) => {
                // The run's ink box: baseline at pos.y; Inter's ascender and
                // descender are within 1em above and 0.3em below.
                let a = at(Abs::zero(), -t.size);
                let b = at(t.width(), t.size * 0.3);
                out.push(Placed {
                    text: Some(t.text.to_string()),
                    image: false,
                    x0: a.x.to_pt(),
                    y0: a.y.to_pt(),
                    x1: b.x.to_pt(),
                    y1: b.y.to_pt(),
                });
            }
            FrameItem::Image(_, size, _) => {
                let (a, b) = (at(Abs::zero(), Abs::zero()), at(size.x, size.y));
                out.push(Placed {
                    text: None,
                    image: true,
                    x0: a.x.to_pt(),
                    y0: a.y.to_pt(),
                    x1: b.x.to_pt(),
                    y1: b.y.to_pt(),
                });
            }
            FrameItem::Shape(..) | FrameItem::Link(..) | FrameItem::Tag(..) => {}
        }
    }
}

fn placed(doc: &typst_layout::PagedDocument, page: usize) -> Vec<Placed> {
    let mut out = Vec::new();
    walk(&doc.pages()[page].frame, Transform::identity(), &mut out);
    out
}

fn inside(p: &Placed, (x, y, w, h): (f64, f64, f64, f64)) -> bool {
    const EPS: f64 = 1e-6;
    p.x0 >= x - EPS && p.y0 >= y - EPS && p.x1 <= x + w + EPS && p.y1 <= y + h + EPS
}

fn cell_of(g: &SheetGeometry, p: &Placed) -> Option<usize> {
    (0..g.per_sheet()).find(|&i| inside(p, g.cell(i)))
}

/// INVARIANTS 1 and 2: every text run and image lies wholly inside one cell,
/// each label's tag and QR code sit in the cell its order and the skip put
/// it in, and the description, however long, never leaves the cell.
#[test]
fn every_label_lies_inside_its_cell_in_order() {
    let g = LabelSheet::Avery5160.geometry();
    let mut ls = labels(40);
    ls[3].description = "An extraordinarily long description that goes on and on, \
        far longer than any label could hold on two lines at any legible size"
        .repeat(3);
    ls[4].tag = "FL-0005-LONGER-TAG".to_string();
    // An absurd tag: cut with an ellipsis (the QR code holds it whole), but
    // still inside its cell.
    ls[6].tag = "X".repeat(60);
    ls[5].serial = Some("S".repeat(80));
    let skip = 7;
    let doc = typeset_labels(&ls, COMPANY, LabelSheet::Avery5160, skip).unwrap();
    assert_eq!(doc.pages().len(), 2);
    for page in 0..2 {
        let items = placed(&doc, page);
        assert!(!items.is_empty());
        for p in &items {
            assert!(
                cell_of(&g, p).is_some(),
                "page {page}: outside every cell: {p:?}"
            );
        }
        // The cells holding something are exactly this page's labels.
        let mut used: Vec<usize> = items.iter().filter_map(|p| cell_of(&g, p)).collect();
        used.sort_unstable();
        used.dedup();
        let expected: Vec<usize> = if page == 0 {
            (skip..30).collect()
        } else {
            (0..17).collect()
        };
        assert_eq!(used, expected, "page {page}");
        // One QR image per label, in its cell.
        let images: Vec<usize> = items
            .iter()
            .filter(|p| p.image)
            .filter_map(|p| cell_of(&g, p))
            .collect();
        assert_eq!(images, expected, "page {page}: one QR per label, in order");
    }
    // Each tag is printed whole in its own cell.
    for (i, l) in ls.iter().enumerate() {
        let (page, pos) = ((skip + i) / 30, (skip + i) % 30);
        if i == 6 {
            assert!(cell_text(&doc, page, pos).contains("XXX…"));
            continue;
        }
        let items = placed(&doc, page);
        let text: String = items
            .iter()
            .filter(|p| cell_of(&g, p) == Some(pos))
            .filter_map(|p| p.text.clone())
            .collect::<Vec<_>>()
            .join(" ");
        assert!(
            text.contains(&l.tag),
            "label {i} at page {page} pos {pos}: {text:?}"
        );
    }
}

/// The text runs in one cell, joined.
fn cell_text(doc: &typst_layout::PagedDocument, page: usize, pos: usize) -> String {
    let g = LabelSheet::Avery5160.geometry();
    placed(doc, page)
        .iter()
        .filter(|p| cell_of(&g, p) == Some(pos))
        .filter_map(|p| p.text.clone())
        .collect::<Vec<_>>()
        .join(" ")
}

/// INVARIANT 6: markup in any field prints literally and never runs.
#[test]
fn text_is_data() {
    let l = AssetLabel {
        tag: "#FL-*1*".to_string(),
        description: "#panic() *bold* $x$".to_string(),
        serial: Some("_u_ @ref".to_string()),
    };
    let doc = typeset_labels(&[l], "<co> #co", LabelSheet::Avery5160, 0).expect("markup is inert");
    let text = cell_text(&doc, 0, 0);
    for s in [
        "#FL-*1*", "#panic()", "*bold*", "$x$", "S/N", "_u_", "@ref", "<co>", "#co",
    ] {
        assert!(text.contains(s), "{s:?} not printed literally in {text:?}");
    }
    // Each markup string alone, in every field, still renders.
    for m in [
        "#panic()", "*", "_", "$", "[", "]", "\\", "#", "<a>", "@a", "//", "/*", "\"", "`",
    ] {
        let l = AssetLabel {
            tag: m.to_string(),
            description: m.to_string(),
            serial: Some(m.to_string()),
        };
        asset_labels_pdf(&[l], m, LabelSheet::Avery5160, 0)
            .unwrap_or_else(|e| panic!("{m:?}: {e}"));
    }
}

/// INVARIANT 1: an overlong description is shortened with an ellipsis;
/// a short one prints whole.
#[test]
fn long_text_is_ellipsized_short_text_is_whole() {
    let long = "word ".repeat(80);
    let ls = [
        AssetLabel {
            tag: "FL-0001".into(),
            description: long.clone(),
            serial: None,
        },
        AssetLabel {
            tag: "FL-0002".into(),
            description: "MacBook Air".into(),
            serial: None,
        },
    ];
    let doc = typeset_labels(&ls, COMPANY, LabelSheet::Avery5160, 0).unwrap();
    let t0 = cell_text(&doc, 0, 0);
    assert!(t0.contains('…'), "{t0:?}");
    assert!(t0.matches("word").count() < 80);
    let t1 = cell_text(&doc, 0, 1);
    assert!(t1.contains("MacBook Air") && !t1.contains('…'), "{t1:?}");
    assert!(!t1.contains("S/N"), "no serial, no S/N line: {t1:?}");
}

/// Rasterize the SVG the template receives (qrcode's renderer: one
/// background rect, then `M{x} {y}h{w}v{h}H{x}V{y}` per dark run).
fn rasterize(svg: &str, scale: usize) -> (usize, Vec<bool>) {
    let attr = |name: &str| -> usize {
        let at = svg.find(&format!(" {name}=\"")).unwrap() + name.len() + 3;
        svg[at..].split('"').next().unwrap().parse().unwrap()
    };
    let (w, h) = (attr("width"), attr("height"));
    assert_eq!(w, h);
    let d = svg
        .split(" d=\"")
        .nth(1)
        .unwrap()
        .split('"')
        .next()
        .unwrap();
    let mut dark = vec![false; w * w];
    for run in d.split('M').filter(|s| !s.is_empty()) {
        let nums: Vec<usize> = run
            .split(|c: char| c.is_ascii_alphabetic() || c == ' ')
            .filter(|s| !s.is_empty())
            .map(|s| s.parse().unwrap())
            .collect();
        let [x, y, rw, rh, ..] = nums[..] else {
            panic!("bad run {run:?}")
        };
        for yy in y..y + rh {
            for xx in x..x + rw {
                dark[yy * w + xx] = true;
            }
        }
    }
    let n = w * scale;
    let px = (0..n * n)
        .map(|i| dark[(i / n / scale) * w + (i % n) / scale])
        .collect();
    (n, px)
}

/// INVARIANT 4: the QR SVG each label prints decodes, with an independent
/// decoder (rqrr), to exactly its tag; and it is the SVG the template gets.
#[test]
fn qr_decodes_to_exactly_the_tag() {
    let tags = [
        "FL-0001",
        "FL-0002",
        "FL-9999",
        "A",
        "fl-0001 lower case & symbols: /?#",
        "FL-0005-WITH-A-RATHER-LONG-TAG",
    ];
    for tag in tags {
        let svg = qr_svg(tag).unwrap();
        let (n, px) = rasterize(&svg, 4);
        let mut img = rqrr::PreparedImage::prepare_from_bitmap(n, n, |x, y| px[y * n + x]);
        let grids = img.detect_grids();
        assert_eq!(grids.len(), 1, "{tag:?}: one code");
        let (meta, content) = grids[0].decode().unwrap();
        assert_eq!(content, tag);
        assert_eq!(meta.ecc_level, 0, "{tag:?}: level M (rqrr numbers M as 0)");
    }
    let data = template_data(&[label(1)], COMPANY, LabelSheet::Avery5160, 0).unwrap();
    assert_eq!(data["pages"][0][0]["qr"], qr_svg("FL-0001").unwrap());
}

/// The sample sheet, for eyeballing: writes `$LABELS_SAMPLE` (a path).
/// Run with `LABELS_SAMPLE=/path/out.pdf cargo test -p ledger-import --test
/// labels -- --ignored sample`.
#[test]
#[ignore = "writes a sample PDF for a human to look at"]
fn sample() {
    let out = std::env::var("LABELS_SAMPLE").expect("set LABELS_SAMPLE to the output path");
    let ls = [
        AssetLabel {
            tag: "FL-0001".into(),
            description: "Framework Laptop 13 (Ryzen 7 7840U, 64 GB, 4 TB)".into(),
            serial: Some("FRANBMCP0A1234".into()),
        },
        AssetLabel {
            tag: "FL-0002".into(),
            description: "MacBook Air".into(),
            serial: None,
        },
        AssetLabel {
            tag: "FL-0003".into(),
            description: "Studio monitor arm with dual gas springs, cable management channel, \
                VESA 75/100 adapter plate, desk clamp and grommet mount, and a spare set of \
                every bolt the box came with"
                .into(),
            serial: Some("ARM-2026-0099887766".into()),
        },
    ];
    let pdf = asset_labels_pdf(&ls, COMPANY, LabelSheet::Avery5160, 0).unwrap();
    std::fs::write(out, pdf).unwrap();
}
