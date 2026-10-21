// The fixed asset register: every asset, its cost, depreciation scheduled
// and posted, and book value, then the totals and the reconciliation to
// the general ledger, then each asset's federal tax figures beside its
// book cost, and the notes that cite the rules.
//
// Everything arrives as data in sys.inputs.data (JSON), laid out by
// ledger-import::asset_register, with amounts already split into exact
// parts. The template only typesets: no arithmetic, and no text is ever
// parsed as markup.
//
// Shape: { company, title, period, footer, columns: [12 headings],
//          tax_title, tax_columns: [9 headings], empty, fmv_mark,
//          lines: [{ tag, description, serial, acquired, in_service, cost,
//                    salvage, life, accumulated_scheduled,
//                    accumulated_posted, book_value, status, disposed,
//                    rule, rule_mark, recorded, fmv_at_contribution|null,
//                    fmv_recorded, contributor_adjusted_basis|null,
//                    tax_basis_723|null, depreciable_basis|null,
//                    built_in_gain|null, method|null, source|null }],
//          totals: [cost, scheduled, posted, book_value],
//          tax_totals: [tax_basis_723, depreciable_basis],
//          missing: text|null, notes: [{ mark, text }],
//          reconciliation: [{ label, amount, difference }] }
// where every amount is { digits: "1,234.56", negative }.

#let d = json(bytes(sys.inputs.data))
#let ink = luma(25)
#let quiet = luma(110)
#let hair = 0.5pt + luma(160)
#let firm = 0.6pt + ink

#set document(title: d.company + " — " + d.title, author: d.company)
#set text(font: ("Inter", "Noto Sans CJK SC", "Noto Sans Arabic UI", "Noto Sans Hebrew", "Noto Sans Thai", "Noto Sans Devanagari"), size: 8pt, fill: ink, number-width: "tabular", hyphenate: false)
#set page(
  paper: "us-letter",
  flipped: true,
  margin: (x: 0.6in, top: 0.7in, bottom: 0.7in),
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

// An amount: negatives in parentheses, the closing one's width reserved on
// every row so decimal points line up; "$" on totals.
#let amount(a, currency: false) = {
  let open = if a.negative { [(] } else { [] }
  let close = if a.negative { [)] } else { hide[)] }
  let sign = if currency [\$] else []
  [#open#sign#a.digits#close]
}

// Columns: 5-10 are numbers (right-aligned).
#let numeric = (5, 6, 7, 8, 9, 10)
#let ncols = d.columns.len()
#let full(body, ..args) = table.cell(colspan: ncols, ..args, body)

#table(
  // Number columns are fixed widths (headings wrap); text takes the rest.
  columns: (auto, 1.4fr, auto, auto, auto, 6.2em, 5.6em, 5.6em, 7em, 7em, 6.2em, 1.2fr),
  stroke: none,
  inset: (x: 3.5pt, y: 3.4pt),
  align: (col, row) => (if col in numeric { right } else { left }) + (if row == 0 { bottom } else { horizon }),
  table.header(
    ..d.columns.map(c => table.cell(
      stroke: (top: firm, bottom: hair),
      text(6.5pt, weight: "bold", tracking: 0.04em, upper(c)),
    )),
  ),
  ..if d.lines.len() == 0 {
    (full(emph(d.empty)),)
  } else {
    d
      .lines
      .map(l => {
        let t(s) = if l.disposed { text(fill: quiet, s) } else { s }
        let a(x) = t(amount(x))
        (
          t(l.tag),
          t(l.description),
          t(l.serial),
          t(l.acquired),
          t(l.in_service),
          a(l.cost),
          a(l.salvage),
          t(l.life),
          a(l.accumulated_scheduled),
          a(l.accumulated_posted),
          a(l.book_value),
          t(l.status),
        )
      })
      .flatten()
  },
  // Totals of the assets held: rule above, double rule beneath.
  ..{
    let s = (top: firm)
    let tot(x) = table.cell(stroke: s, text(weight: "bold", amount(x, currency: true)))
    let blank = table.cell(stroke: s, [])
    (
      table.cell(stroke: s, colspan: 5, text(weight: "bold", "Total, assets held")),
      tot(d.totals.at(0)),
      blank,
      blank,
      tot(d.totals.at(1)),
      tot(d.totals.at(2)),
      tot(d.totals.at(3)),
      blank,
    )
  },
  full(
    inset: (x: 0pt, y: 0pt),
    stack(spacing: 1.6pt, line(length: 100%, stroke: firm), line(length: 100%, stroke: firm)),
  ),
)

// The reconciliation to the books, drawn beside the notes at the end.
#let reconciliation = block(breakable: false, {
  text(weight: "semibold", "Reconciliation to the books")
  v(2pt)
  table(
    columns: (auto, 9em),
    stroke: none,
    inset: (x: 4pt, y: 3.4pt),
    align: (left + horizon, right + horizon),
    ..d
      .reconciliation
      .map(x => {
        if x.difference {
          let s = (top: hair)
          (
            table.cell(stroke: s, text(weight: "bold", x.label)),
            table.cell(stroke: s, text(weight: "bold", amount(x.amount, currency: true))),
            table.cell(colspan: 2, inset: 2pt, []),
          )
        } else {
          (pad(left: 1.4em, x.label), amount(x.amount))
        }
      })
      .flatten(),
  )
})

// Federal tax figures, asset by asset, beside the book cost they may
// differ from; each rule marked with the number of the note that cites
// it, a value read as the book cost marked too; under the description,
// the §704(c) method and who gave the facts.
#let tax_numeric = (3, 4, 5, 6, 7, 8)
#let tcols = d.tax_columns.len()
#let maybe(x) = if x == none { [] } else { amount(x) }
#v(12pt)
#text(weight: "semibold", d.tax_title)
#v(2pt)
#table(
  // Each amount column wide enough for its heading's longest word.
  columns: (auto, 1fr, auto, 6.2em, 8.6em, 7.6em, 6.4em, 7.2em, 7.6em),
  stroke: none,
  inset: (x: 3.5pt, y: 3.4pt),
  align: (col, row) => (if col in tax_numeric { right } else { left }) + (if row == 0 { bottom } else { horizon }),
  table.header(
    ..d.tax_columns.enumerate().map(((i, c)) => table.cell(
      stroke: (top: firm, bottom: hair),
      text(6.5pt, weight: "bold", tracking: 0.04em, {
        upper(c)
        if i == tcols - 1 { super("4") }
      }),
    )),
  ),
  ..if d.lines.len() == 0 {
    (table.cell(colspan: tcols, emph(d.empty)),)
  } else {
    d
      .lines
      .map(l => {
        let t(s) = if l.disposed { text(fill: quiet, s) } else { s }
        let rule = if l.recorded {
          [#l.rule#super(l.rule_mark)]
        } else if l.disposed {
          emph(l.rule)
        } else {
          text(fill: rgb("#9a5b00"), emph(l.rule))
        }
        let fmv = if l.fmv_at_contribution == none {
          []
        } else if l.fmv_recorded {
          amount(l.fmv_at_contribution)
        } else {
          [#super(d.fmv_mark)#amount(l.fmv_at_contribution)]
        }
        let about = (l.method, l.source).filter(x => x != none)
        (
          t(l.tag),
          t({
            l.description
            for x in about {
              linebreak()
              text(6.5pt, fill: quiet, x)
            }
          }),
          t(rule),
          t(amount(l.cost)),
          t(fmv),
          t(maybe(l.contributor_adjusted_basis)),
          t(maybe(l.tax_basis_723)),
          t(maybe(l.depreciable_basis)),
          t(maybe(l.built_in_gain)),
        )
      })
      .flatten()
  },
  ..{
    let s = (top: firm)
    let blank = table.cell(stroke: s, [])
    (
      table.cell(stroke: s, colspan: 6, text(weight: "bold", "Total, assets held")),
      table.cell(stroke: s, text(weight: "bold", amount(d.tax_totals.at(0), currency: true))),
      table.cell(stroke: s, text(weight: "bold", amount(d.tax_totals.at(1), currency: true))),
      blank,
    )
  },
  table.cell(
    colspan: tcols,
    inset: (x: 0pt, y: 0pt),
    stack(spacing: 1.6pt, line(length: 100%, stroke: firm), line(length: 100%, stroke: firm)),
  ),
)
#if d.missing != none {
  v(4pt)
  text(fill: rgb("#9a5b00"), emph(d.missing))
}

// Notes: book value against tax basis, and each rule's citation.
#let notes = {
  set text(7.5pt, fill: luma(70))
  set par(leading: 0.5em)
  text(weight: "semibold", fill: ink, "Notes")
  block(above: 6pt, d.notes.at(0).text)
  for n in d.notes.slice(1) {
    block(above: 4pt, grid(columns: (1.2em, 1fr), super(n.mark), n.text))
  }
}

// The reconciliation and the notes, side by side.
#v(12pt)
#grid(columns: (auto, 1fr), column-gutter: 32pt, reconciliation, pad(top: 1pt, notes))
