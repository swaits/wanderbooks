// Asset label sheets: one tag per label, with its QR code.
//
// Everything arrives as data in sys.inputs.data (JSON), built by
// ledger-import::labels: the sheet geometry (in points, from
// docs/spec/sources/avery-5160.md), and one entry per sheet position. The
// template only typesets: no label text is ever parsed as markup, and every
// label is drawn inside a clipped box the size of its cell, so nothing can
// print past its edge.
//
// Shape: { company, geometry: { page_width, page_height, label_width,
//          label_height, left, top, pitch_x, pitch_y, columns, rows, inset },
//          pages: [[null | { tag, description, serial: null | str, qr: svg }]] }
// Each inner list is one sheet, positions in reading order (left to right,
// then top to bottom); null is a position left blank.

#let d = json(bytes(sys.inputs.data))
#let g = d.geometry
#let pt1 = 1pt
#let ink = luma(15)
#let quiet = luma(90)

#set document(title: d.company + " — Asset labels", author: d.company)
#set text(font: "Inter", fill: ink, number-width: "tabular", hyphenate: false)
#set par(leading: 0.28em, spacing: 0pt)
#set page(width: g.page_width * pt1, height: g.page_height * pt1, margin: 0pt)

// The inner box every label's content lives in.
#let inset = g.inset * pt1
#let cell-w = g.label_width * pt1
#let cell-h = g.label_height * pt1
// The QR code (its 4-module quiet zone included) fills the label's height.
#let qr-size = cell-h - 2 * inset
#let gap = 4pt
#let text-w = cell-w - 2 * inset - qr-size - gap

// The largest of `sizes` at which `s` fits in `lines` lines of `width`;
// at the smallest size, the longest prefix that fits, ending in "…".
// Text is set as a string, so it is always literal.
#let fit(s, width, sizes, lines, weight: "regular", fill: ink) = context {
  let set-at(size, body) = block(width: width, text(size: size, weight: weight, fill: fill, body))
  let limit(size) = measure(set-at(size, ("X\n" * lines).trim())).height + 0.01pt
  // Fits: wraps into `lines` lines, and no word is wider than the box
  // (an unbreakable word would overflow sideways, not down).
  let fits(size, body) = (
    measure(set-at(size, body)).height <= limit(size)
      and body.split(" ").all(w => measure(text(size: size, weight: weight, w)).width <= width)
  )
  let chosen = sizes.find(size => fits(size, s))
  if chosen != none {
    set-at(chosen, s)
  } else {
    let size = sizes.last()
    let cs = s.clusters()
    // Binary search for the longest prefix that fits with the ellipsis.
    let lo = 0
    let hi = cs.len()
    while lo < hi {
      let mid = calc.quo(lo + hi + 1, 2)
      if fits(size, cs.slice(0, mid).join().trim() + "…") { lo = mid } else { hi = mid - 1 }
    }
    let prefix = if lo == 0 { "" } else { cs.slice(0, lo).join().trim() }
    set-at(size, prefix + "…")
  }
}

#let label(l) = box(width: cell-w, height: cell-h, clip: true, inset: inset, {
  grid(
    columns: (qr-size, gap, text-w),
    rows: (qr-size,),
    image(bytes(l.qr), format: "svg", width: qr-size, height: qr-size),
    [],
    align(horizon, stack(
      spacing: 2.2pt,
      fit(d.company, text-w, (6pt, 5.5pt, 5pt), 1, fill: quiet),
      fit(l.tag, text-w, (15pt, 13pt, 11pt, 9pt, 8pt, 7pt, 6pt), 1, weight: "bold"),
      fit(l.description, text-w, (7.5pt, 7pt, 6.5pt, 6pt), 2),
      if l.serial != none { fit("S/N " + l.serial, text-w, (6.5pt, 6pt, 5.5pt, 5pt), 1, fill: quiet) },
    )),
  )
})

#for (i, sheet) in d.pages.enumerate() {
  if i > 0 { pagebreak() }
  for (p, l) in sheet.enumerate() {
    if l != none {
      let col = calc.rem(p, g.columns)
      let row = calc.quo(p, g.columns)
      place(top + left, dx: (g.left + col * g.pitch_x) * pt1, dy: (g.top + row * g.pitch_y) * pt1, label(l))
    }
  }
}
