// An invoice: what the company billed a customer, one US-letter page or
// more.
//
// Everything arrives as data in sys.inputs.data (JSON), built by
// ledger-import::invoice_pdf, every amount already formatted. The template
// only typesets: it does no arithmetic, and no text is ever parsed as
// markup (a description holding Typst syntax prints literally).
//
// Shape: { title: "INVOICE", number, issue_date, due_date, terms: null | str,
//          seller: { name, lines: [str] }, bill_to: { name, lines: [str] },
//          lines: [{ description, amount }], total,
//          settled: [{ label, amount }], balance_label, balance,
//          stamp: null | str, stamp_draft: bool, stamp_line: null | str,
//          note: null | str }

#let d = json(bytes(sys.inputs.data))
#let ink = luma(25)
#let quiet = luma(110)
#let hair = 0.5pt + luma(170)
#let firm = 0.8pt + ink

#set document(title: d.seller.name + " — " + d.number, author: d.seller.name)
#set text(font: ("Inter", "Noto Sans CJK SC", "Noto Sans Arabic UI", "Noto Sans Hebrew", "Noto Sans Thai", "Noto Sans Devanagari"), size: 10pt, fill: ink, number-width: "tabular")
#set par(leading: 0.45em, spacing: 0.6em)
#set page(
  paper: "us-letter",
  margin: (x: 0.8in, top: 0.8in, bottom: 0.9in),
  footer: context {
    set text(7.5pt, fill: quiet)
    [#d.seller.name · #d.number #h(1fr) Page #counter(page).display() of #counter(page).final().first()]
  },
)

// A block of lines, each set as a string (always literal).
#let lines(ls) = for l in ls [#l \ ]

#grid(
  columns: (1fr, auto),
  align: (left + top, right + top),
  [
    #text(15pt, weight: "bold", d.seller.name) \
    #set text(fill: quiet)
    #lines(d.seller.lines)
  ],
  [
    #text(22pt, weight: "bold", tracking: 0.08em, d.title) \
    #v(2pt)
    #text(11pt, weight: "semibold", d.number)
  ],
)

#v(0.35in)
#grid(
  columns: (1fr, auto),
  column-gutter: 24pt,
  [
    #text(8pt, weight: "semibold", fill: quiet, tracking: 0.06em, "BILL TO") \
    #v(2pt)
    #text(weight: "semibold", d.bill_to.name) \
    #lines(d.bill_to.lines)
  ],
  grid(
    columns: (auto, auto),
    column-gutter: 14pt,
    row-gutter: 6pt,
    align: (right, left),
    text(fill: quiet, "Invoice date"), d.issue_date,
    text(fill: quiet, "Due date"), text(weight: "semibold", d.due_date),
    ..if d.terms != none { (text(fill: quiet, "Terms"), d.terms) } else { () },
  ),
)

#v(0.3in)
#table(
  columns: (1fr, auto),
  stroke: none,
  inset: (x: 0pt, y: 7pt),
  align: (left, right),
  table.hline(stroke: firm),
  table.header(
    text(8pt, weight: "semibold", fill: quiet, tracking: 0.06em, "DESCRIPTION"),
    text(8pt, weight: "semibold", fill: quiet, tracking: 0.06em, "AMOUNT"),
  ),
  table.hline(stroke: hair),
  ..d.lines.map(l => (l.description, l.amount)).flatten(),
  table.hline(stroke: firm),
)

// The stamp, if any, sits in the space left of the totals, as a rubber
// stamp would.
#let stamp = if d.stamp == none { none } else {
  let ink-stamp = if d.stamp_draft { luma(130) } else { rgb("#c62828") }
  rotate(-10deg, reflow: true, box(
    stroke: 2.5pt + ink-stamp,
    inset: (x: 12pt, y: 6pt),
    radius: 4pt,
    align(center, {
      text(24pt, weight: "bold", tracking: 0.1em, fill: ink-stamp, d.stamp)
      if d.stamp_line != none {
        linebreak()
        text(9pt, weight: "semibold", fill: ink-stamp, d.stamp_line)
      }
    }),
  ))
}

#v(6pt)
#grid(
  columns: (1fr, auto),
  column-gutter: 16pt,
  align: (center + horizon, right + top),
  stamp,
  grid(
    columns: (auto, 1.2in),
    column-gutter: 16pt,
    row-gutter: 8pt,
    align: (right, right),
    text(fill: quiet, "Total"), d.total,
    ..d.settled.map(s => (text(fill: quiet, s.label), s.amount)).flatten(),
    grid.hline(stroke: hair),
    text(weight: "bold", d.balance_label), text(12pt, weight: "bold", d.balance),
  ),
)

#if d.note != none {
  v(0.3in)
  text(8.5pt, fill: quiet, d.note)
}
