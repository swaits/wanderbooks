// A financial statement: P&L, balance sheet, cash flow, trial balance, or
// members' equity, in US small-business (QuickBooks-style) presentation.
//
// Everything arrives as data in sys.inputs.data (JSON), already laid out by
// ledger-import::report, with amounts already split into exact parts. The
// template only typesets: no arithmetic happens here, and no label is ever
// parsed as markup.
//
// Shape: { company, title, period, footer, columns: ["Total"] | [...],
//          rows: [{ kind: "section" | "subheading" | "line" | "detail"
//                       | "subtotal" | "total" | "spacer",
//                   label, rule (rule above), currency ("$" on amounts),
//                   amounts: [null | { digits: "1,234.56", negative }] }] }
//
// Amounts: Inter's tabular figures; negatives in parentheses with the
// closing parenthesis hanging (its width is reserved on every row, so
// decimal points always line up); "$" attached on subtotals and totals.

#let d = json(bytes(sys.inputs.data))
#let ncols = d.columns.len()
// Wide statements (by month, with comparisons) turn the page and, past
// seven columns, set smaller and tighter so a year of months fits.
#let wide = ncols > 4
#let dense = ncols > 7
#let ink = luma(25)
#let quiet = luma(110)
#let hair = 0.5pt + luma(160)
#let firm = 0.6pt + ink

#set document(title: d.company + " — " + d.title, author: d.company)
#set text(font: ("Inter", "Noto Sans CJK SC", "Noto Sans Arabic UI", "Noto Sans Hebrew", "Noto Sans Thai", "Noto Sans Devanagari"), size: if dense { 7pt } else { 9pt }, fill: ink, number-width: "tabular")
#set page(
  paper: "us-letter",
  flipped: wide,
  margin: (x: 0.75in, top: 0.75in, bottom: 0.75in),
  header: context {
    if counter(page).get().first() > 1 {
      set text(7.5pt, fill: quiet)
      [#d.company · #d.title #h(1fr) #d.period]
    }
  },
  footer: context {
    set text(7.5pt, fill: quiet)
    grid(
      columns: (1fr, auto, 1fr),
      [],
      d.footer,
      align(right)[Page #counter(page).display() of #counter(page).final().first()],
    )
  },
)

#align(center)[
  #text(15pt, fill: luma(45), d.company) \
  #v(4pt)
  #text(10.5pt, weight: "bold", d.title) \
  #v(2pt)
  #d.period
]
#v(12pt)

#let amount(a, currency) = {
  if a == none { return [] }
  let open = if a.negative { [(] } else { [] }
  let close = if a.negative { [)] } else { hide[)] }
  let sign = if currency and a.at("percent", default: false) == false [\$] else []
  [#open#sign#a.digits#close]
}

#let full(body, ..args) = table.cell(colspan: ncols + 1, ..args, body)

#let row-cells(r) = {
  let bold = r.kind == "subtotal" or r.kind == "total"
  let stroke = if r.rule { (top: if r.kind == "total" { firm } else { hair }) } else { none }
  // A detail (a transaction under its kind's line) sits further in and
  // quieter: its amount is part of the line above, not of the section.
  let detail = r.kind == "detail"
  let quietly(body) = if detail { text(size: 0.9em, fill: quiet, body) } else { body }
  let label = if r.kind == "line" { pad(left: 1.4em, r.label) } else if detail { pad(left: 2.8em, quietly(r.label)) } else { r.label }
  let label = if bold { text(weight: "bold", label) } else { label }
  (
    table.cell(stroke: stroke, label),
    ..r.amounts.map(a => table.cell(stroke: stroke, quietly(amount(a, r.currency)))),
  )
}

#table(
  columns: (1fr,) + (if dense { 6.6em } else { 8em },) * ncols,
  stroke: none,
  inset: (x: 4pt, y: 3.6pt),
  align: (left + horizon,) + (right + horizon,) * ncols,
  table.header(
    table.cell(stroke: (top: firm, bottom: hair), []),
    ..d.columns.map(c => table.cell(
      stroke: (top: firm, bottom: hair),
      text(if dense { 6pt } else { 7.5pt }, weight: "bold", tracking: 0.04em, upper(c)),
    )),
  ),
  ..d
    .rows
    .map(r => {
      if r.kind == "spacer" {
        (full(v(2pt), inset: 2pt),)
      } else if r.kind == "section" {
        (full(text(weight: "semibold", r.label)),)
      } else if r.kind == "subheading" {
        (full(pad(left: 1.4em, emph(r.label))),)
      } else if r.kind == "total" {
        // The double rule under a statement total, full width.
        let double = full(
          inset: (x: 0pt, y: 0pt),
          stack(
            spacing: 1.6pt,
            line(length: 100%, stroke: firm),
            line(length: 100%, stroke: firm),
          ),
        )
        row-cells(r) + (double,)
      } else {
        row-cells(r)
      }
    })
    .flatten(),
)
