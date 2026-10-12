// A financial statement: P&L, balance sheet, or cash flow.
//
// Everything arrives as data in sys.inputs.data (JSON), already laid out by
// ledger-import::report, with amounts already split into exact parts. The
// template only typesets: no arithmetic happens here, and no label is ever
// parsed as markup.
//
// Shape: { company, title, period, footer,
//          rows: [{ kind: "section" | "line" | "subtotal" | "total"
//                       | "spacer", label, rule (single rule above),
//                   amount: null | { digits: "1,234.56", negative, zero,
//                                    currency } }] }
//
// Amounts follow US accounting presentation: tabular figures, "$" pinned
// to the left of the amount column on the first line of each section and on
// totals, negatives in parentheses with the closing parenthesis hanging
// (its width is reserved on every row, so decimal points always line up),
// and zero as a dash in the ones position. Amounts are never bold: bold
// figures are wider, which would knock totals out of the decimal column —
// the rules mark subtotals and totals instead.

#let d = json(bytes(sys.inputs.data))

#set document(title: d.company + " — " + d.title, author: d.company)
#set text(
  font: "Libertinus Serif",
  size: 10.5pt,
  number-type: "lining",
  number-width: "tabular",
)
#set page(
  paper: "us-letter",
  margin: (x: 1in, top: 0.9in, bottom: 0.8in),
  header: context {
    if counter(page).get().first() > 1 {
      set text(8.5pt, fill: luma(110))
      [#d.company — #d.title #h(1fr) #d.period]
    }
  },
  footer: context {
    set text(8pt, fill: luma(110))
    [#d.footer #h(1fr) Page #counter(page).display() of #counter(page).final().first()]
  },
)

#align(center)[
  #text(15pt, weight: "bold", d.company) \
  #v(2pt)
  #text(12.5pt, d.title) \
  #v(1pt)
  #text(10pt, style: "italic", d.period)
]
#v(16pt)

// Two lines under a statement total, the accounting double rule, set
// clear of the descenders.
#let double-rule(body) = stack(
  body,
  v(2.4pt),
  line(length: 100%, stroke: 0.5pt),
  v(1.4pt),
  line(length: 100%, stroke: 0.5pt),
)

#let amount(r) = {
  let a = r.amount
  if a == none { return [] }
  let open = if a.negative { [(] } else { [] }
  let close = if a.negative { [)] } else { hide[)] }
  let body = if a.zero { [–#hide[.00]] } else { [#open#a.digits] }
  grid(
    columns: (auto, 1fr),
    align: (left, right),
    if a.currency [\$] else [],
    [#body#close],
  )
}

#table(
  columns: (1fr, 9em),
  stroke: none,
  inset: (x: 0pt, y: 3.2pt),
  align: (left + horizon, right + horizon),
  ..d
    .rows
    .map(r => {
      if r.kind == "spacer" {
        (table.cell(colspan: 2, v(3pt)),)
      } else if r.kind == "section" {
        (table.cell(colspan: 2, text(weight: "bold", r.label)),)
      } else if r.kind == "line" {
        (pad(left: 1.4em, r.label), amount(r))
      } else if r.kind == "subtotal" {
        (
          text(weight: "bold", r.label),
          table.cell(
            stroke: if r.rule { (top: 0.5pt) } else { none },
            amount(r),
          ),
        )
      } else {
        (
          align(top, text(weight: "bold", r.label)),
          table.cell(
            stroke: (top: 0.5pt),
            double-rule(amount(r)),
          ),
        )
      }
    })
    .flatten(),
)
