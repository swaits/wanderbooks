# Specification sources

wanderbooks' rules come from documents, not from anyone's memory. This
directory holds the authorities the engine's specs cite (`docs/spec/rules.md`
is the rule catalog), and every spec block in the code carries a `SOURCE:`
line pointing into it.

## The authority stack

There is no single "bible" of US bookkeeping. Authority is layered, and the
layers differ in what they cover and in whether we may copy them:

| Layer | What it governs | Copyright | Here |
|---|---|---|---|
| **Pacioli (1494), Geijsbeek translation (1914)** | The mechanics of double entry: debits equal credits, the trial balance, one money, writing only forward, correcting by opposite entry, closing into capital. No modern standard restates these; they predate every regulator. | Public domain | Vendored verbatim: `sources/pacioli-1494.md` |
| **IRS: Internal Revenue Code, Treasury Regulations, publications, form instructions** | Records and supporting documents, retention, accounting periods and methods, information returns (1099), capitalization (de minimis, 12-month rule), partners' capital, tax basis of purchased and contributed property (`sources/irs-partnership-basis.md`), payroll taxes, rounding on returns, return line items for tax categories (`sources/irs-tax-lines.md`) | Public domain (17 U.S.C. §105; IRS.gov's content policy) | Vendored verbatim: `sources/irs-*.md`, `sources/us-public-domain-status.md` |
| **File formats: Intuit (QuickBooks Online bank CSV, QuickBooks Desktop IIF), ECMA-376 Office Open XML, Microsoft Excel limits, Avery label sheets** | What `ledger-import` reads and writes: bank/card CSV layouts and signs, IIF journal entries, how XLSX stores numbers and Excel's 15-digit precision, asset-label sheet geometry (Avery 5160) | © Intuit Inc.; © Ecma International; © Microsoft; © Avery Products Corporation | Cited with short quotes, URLs, and hashes: `sources/intuit-bank-csv.md`, `sources/intuit-iif.md`, `sources/xlsx-numbers.md`, `sources/avery-5160.md` |
| **FASB Accounting Standards Codification (ASC)** | Authoritative US GAAP: cash flows (ASC 230), revenue and principal vs agent (ASC 606), foreign currency (ASC 830), credit losses and aging (ASC 326), property and depreciation (ASC 360) | © Financial Accounting Foundation | Cited by paragraph with short quotes only (in `rules.md`; the paragraphs read, with where and when, in `sources/fasb-asc.md` and below) |
| **FASB Concepts Statement No. 8, Chapter 4 (2021)** | Definitions of the five elements (it replaced Concepts Statement 6) | © Financial Accounting Foundation; non-authoritative | Cited with short quotes only |
| **Textbooks (OpenStax *Principles of Accounting, Vol. 1*)** | Conventions no standard states: normal balances, closing-entry mechanics, straight-line depreciation arithmetic | CC BY-NC-SA 4.0 (incompatible with this repo's license) | Cited with short quotes and attribution only |
| **Convention or decision** | Where no authority speaks: aging bucket edges, cent rounding among members, closed-period locks | — | Stated as CONVENTION or DECISION, with who decided and when |

## How to read a SOURCE line

```text
SOURCE: Pacioli ch. 36 (sources/pacioli-1494.md); IRS Pub. 583 (sources/irs-records.md)
SOURCE: ASC 830-20-30-1 (cited, rules.md B-3)
SOURCE: DECISION (the owner, 2026-10-13)
SOURCE: CONVENTION (QuickBooks aging); nearest authority ASC 326-20-55-38 (illustrative)
```

A rule whose only source is CONVENTION or DECISION is a candidate for a
human to confirm or overturn; nothing else is.

## Rules added later never lock out older books

When a rule is tightened (a check added to a command), books written
before it must still open. Journal replay and the SQLite cross-check read
history through the engine's `replay_*` commands, which skip checks added
later; new commands go through the checked ones. Pinned by
`books_written_under_older_rules_still_open` (ledger-store).

## Vendoring rules

- Public-domain texts are copied **verbatim** (whitespace normalized), each
  with its URL, retrieval date, and the SHA-256 of the retrieved file, so
  anyone can check the excerpt against the original.
- Copyrighted sources are cited by paragraph, quoted briefly, never copied.
- When an authority changes (the IRS revises a publication; a statute is
  amended), add the new text beside the old with its effective date. Rules
  that depend on dates (1099 thresholds) must say which text applies when.
- Rules that change with the law are registered in
  `crates/ledger-core/src/rules.rs` with the years they are known for and
  a review date. Past what it knows, the engine refuses rather than
  guesses; from the review date, the MCP digest and the viewer flag the
  rule. Whoever updates one (human or agent) vendors the new official
  text here, extends the rule, and sends the change upstream: a pull
  request at https://github.com/swaits/wanderbooks (best), or an issue.

## Open: sources we could not verify

Read on 2026-10-13 through Deloitte's free DART roadmaps, which reproduce
ASC text: ASC 230-10-45-16 and -28, 326-20-55-37/38, 606-10-25-23, 45-2,
55-37/38, 830-20-30-1. Read directly in the ASC basic view on 2026-10-14
(`sources/fasb-asc.md`): 360-10-35-4, 340-10-05-2/4/5, 210-10-45-1/2/5,
230-10-45-7/10/12/13/14/15, 606-10-55-36 through 55-38. Read on
2026-10-20 where public sources reproduce the Codification (EY's FRD on
the statement of cash flows, July 2026; FASB ASUs 2016-15 and 2016-18;
Deloitte's public DART Roadmap): ASC 230-10-10-1, 45-1, 45-4, 45-5,
45-8, 45-9, 45-12(c), 45-13(c), 45-14(a), 45-15(a)/(c), 45-16(a)/(b),
45-17, 45-24 to 45-26, 45-28 to 45-31, 50-3 to 50-8, and 830-230-45-1;
each checked again against the same sources in the P3 review (numbers,
pages and wording hold). Open: 105-10-05-1 to 05-3, read only in their
2009 text (ASU 2009-01, and as Wiley GAAP 2020 quotes them); their
current wording is unconfirmed. Also open (P3 surface review, 2026-10-21): ASC 230-10-45-15(f) (debt
issue costs, cited by B-14 rule 11) is in neither list above; ASC
606-10-25-31, 25-32 and 55-20 (B-16's over-time method) are not yet read;
B-15 rule 6 cites ASC 250 with no paragraph; B-17's 606-10-55-39 is in
neither list. Each is to be read in the ASC basic view and listed in
`sources/fasb-asc.md` before it is relied on further. Also open (plan
P4b, 2026-10-23): B-38 cites ASC 310-10, 405-20-40-1 and 606-10-45-4,
none yet read in the ASC basic view; B-38 keeps the CONVENTION label
until they are vendored with a SHA-256, as P4b.16 lists with the sources
B-39 to B-41 will need.
