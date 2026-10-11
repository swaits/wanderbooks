# Capability parity — GSheet+GDrive → wanderbooks

The parity target for the AI-first phase is **not QuickBooks**. It is the
system actually in use today: the "Wanderland Labs LLC — Books" Google Sheet
(double-entry accounts with live P&L, Balance Sheet, and Cash Flow tabs, plus
the Transaction Journal) together with the Drive folder, operated through an
AI assistant. Every capability below must be reachable through MCP tools
before the sheet retires.

Statuses: `covered` (the engine already does it), `specced` (designed in
this phase, not yet built), `future` (on the engine roadmap).

| # | Today's capability | wanderbooks equivalent | Status |
|---|---|---|---|
| 1 | Record a double-entry transaction (Transaction Journal row) | `books.draft_entry` / `add_line` / `post_entry` (engine: `Ledger`) | specced |
| 2 | Chart of accounts | engine + `books.chart` (with live balances) | specced |
| 3 | Live P&L tab | engine + `books.profit_and_loss` | specced |
| 4 | Live Balance Sheet tab | engine + `books.balance_sheet` | specced |
| 5 | Live Cash Flow tab | engine R-FUTURE-3 | future |
| 6 | Transaction Journal as append-only log | engine posting log + `books.search_entries` | specced |
| 7 | Receipt storage in Drive | blob store + `books.attach_receipt` / `link_receipt` | specced |
| 8 | Document filing by category (01 Formation / 02 Legal / 03 Tax / 04 Accounting) | `books.file_document` | specced |
| 9 | Ad-hoc questions ("what did we spend on X?") | `search_entries` + reports; the AI itself is the query layer | specced |
| 10 | Human review of what the assistant recorded | `books.digest` | specced |
| 11 | CPA-ready export of the books | `books.export` (csv/json) | specced |
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
- Row 12 is deliberately excluded. Email filing is a mail problem, not a
  books problem; it stays where it is.

## Cutover rule

The sheet does not retire on a date. It retires when **all three** hold:

1. Every `specced` row above is `covered` (built, tested, golden-covered
   where reports are involved).
2. A full month of parallel run — every real company transaction recorded in
   both systems — diffs clean.
3. Stephen says so.

The parallel run is the acceptance test. Until then the sheet is the system
of record and wanderbooks is the shadow ledger; after, it inverts.
