# Capability parity — GSheet+GDrive → wanderbooks

The parity target for the AI-first phase is **not QuickBooks**. It is the
system actually in use today: the "the first company — Books" Google Sheet
(double-entry accounts with live P&L, Balance Sheet, and Cash Flow tabs, plus
the Transaction Journal) together with the Drive folder, operated through an
AI assistant. Every capability below must be reachable through MCP tools
before the sheet retires.

Statuses: `covered` (built and tested), `partial` (built, known gaps listed),
`specced` (designed, not yet built), `future` (on the roadmap).

| # | Today's capability | wanderbooks equivalent | Status |
|---|---|---|---|
| 1 | Record a double-entry transaction (Transaction Journal row) | `books.draft_entry` / `add_line` / `post_entry` (engine: `Ledger`) | covered |
| 2 | Chart of accounts | engine + `books.chart` (with live balances) | covered |
| 3 | Live P&L tab | engine + `books.profit_and_loss` | covered |
| 4 | Live Balance Sheet tab | engine + `books.balance_sheet` | covered |
| 5 | Live Cash Flow tab | engine `cash_flow` (direct, with the reconciliation schedule) + `books.cash_flow` | covered |
| 6 | Transaction Journal as append-only log | engine posting log + `books.search_entries` | covered |
| 7 | Receipt storage in Drive | blob store + `books.attach_receipt` / `link_receipt` / `unlink_receipt` / `withdraw_attachment` | covered |
| 8 | Document filing by category (01 Formation / 02 Legal / 03 Tax / 04 Accounting) | `books.file_document` | covered |
| 9 | Ad-hoc questions ("what did we spend on X?") | `search_entries` + reports; the AI itself is the query layer | covered |
| 10 | Human review of what the assistant recorded | `books.digest` | covered |
| 11 | CPA-ready export of the books | `books.export` (csv/json/xlsx/pdf/iif, each handed over as a download: D-46) | covered |
| 12 | Company email filing (every company email → Drive folder) | out of books scope; the existing mail-filing automation keeps running unchanged | — |

## Notes

- Row 9 is the quiet superpower of the AI-first design. The sheet answers
  questions with formulas the human must write; the MCP design answers them
  with an agent that already knows how to read a trial balance. The
  capability is "ask anything about the books," and the tool surface above
  is what makes it true.
- Row 10 (`books.digest`) has no sheet equivalent — the sheet can't tell you
  what changed. This is parity *plus*: the new system is more reviewable
  than the old one, which is the point of the exercise.
- Projects (rules.md B-37) have no sheet equivalent either: a class with
  its facts, created and changed by the agent (`books.add_project`,
  `books.update_project`), read by the viewer's Projects pages and by
  `books.projects` / `books.project` (D-28: the agent writes, the viewer
  reads).
- Row 12 is deliberately excluded. Email filing is a mail problem, not a
  books problem; it stays where it is.

## Cutover: done 2026-10-20

The three conditions below were met and the owner said so: the books were
adopted into the server's data root as the first company's live books on
2026-10-20 (P2 slice 9), the Sheet is frozen as of 2026-10-11 with the
final `verify-sheet` run filed as a workpaper, and the old copies moved
into the data root's `legacy/` (Q-3, D-2). The rule is kept for the record
and for the next company.

## Cutover rule (as it was)

The sheet does not retire on a date. It retires when **all three** hold:

1. Every `specced` row above is `covered` (built, tested, golden-covered
   where reports are involved).
2. A full month of parallel run — every real company transaction recorded in
   both systems — diffs clean.
3. The owner says so.

The parallel run is the acceptance test. Until then the sheet is the system
of record and wanderbooks is the shadow ledger; after, it inverts.
