// An audit workpaper: what was checked, through which date and which
// books' state, and every finding with its code and severity.
//
// Everything arrives as data in sys.inputs.data (JSON), built by
// ledger-import::workpaper. The template only typesets: no arithmetic, and
// no text is ever parsed as markup. It holds no clock: the same data gives
// the same bytes.
//
// Shape: { company, title, scope, through, through_seq, program_version,
//          tb_hash, findings_sha256, summary: str,
//          findings: [{ severity, code, subject, message }],
//          checks: [{ code, severity, predicate }] }

#let d = json(bytes(sys.inputs.data))
#let ink = luma(25)
#let quiet = luma(110)
#let hair = 0.5pt + luma(160)

#set document(title: d.company + " — " + d.title, author: d.company)
#set text(font: ("Inter", "Noto Sans CJK SC", "Noto Sans Arabic UI", "Noto Sans Hebrew", "Noto Sans Thai", "Noto Sans Devanagari"), size: 9pt, fill: ink, number-width: "tabular", hyphenate: false)
#set page(
  paper: "us-letter",
  margin: (x: 0.8in, top: 0.8in, bottom: 0.8in),
  footer: context {
    set text(7.5pt, fill: quiet)
    grid(
      columns: (1fr, auto),
      [#d.program_version · head #d.through_seq],
      align(right)[Page #counter(page).display() of #counter(page).final().first()],
    )
  },
)

#align(center)[
  #text(15pt, fill: luma(45), d.company) \
  #v(4pt)
  #text(10.5pt, weight: "bold", d.title) \
  #v(2pt)
  Through #d.through · scope #d.scope
]
#v(12pt)

#grid(
  columns: (auto, 1fr),
  row-gutter: 5pt,
  column-gutter: 12pt,
  text(fill: quiet)[Books' figure head], [#d.through_seq],
  text(fill: quiet)[Trial balance hash], text(size: 7.5pt)[#d.tb_hash],
  text(fill: quiet)[Findings hash], text(size: 7.5pt)[#d.findings_sha256],
  text(fill: quiet)[Result], [#d.summary],
)
#v(12pt)

#text(10pt, weight: "bold")[Findings]
#v(4pt)
#if d.findings.len() == 0 [
  No findings.
] else [
  #table(
    columns: (auto, auto, auto, 1fr),
    stroke: (x: none, y: hair),
    inset: (x: 4pt, y: 4pt),
    table.header([*Severity*], [*Code*], [*About*], [*Finding*]),
    ..d.findings.map(f => (f.severity, f.code, f.subject, f.message)).flatten(),
  )
]
#v(14pt)

#text(10pt, weight: "bold")[What was checked]
#v(4pt)
#table(
  columns: (auto, auto, 1fr),
  stroke: (x: none, y: hair),
  inset: (x: 4pt, y: 4pt),
  table.header([*Code*], [*Severity*], [*True when it is reported*]),
  ..d.checks.map(c => (c.code, c.severity, c.predicate)).flatten(),
)
