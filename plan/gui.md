# ledger-gui — design

Phase 4 of the roadmap. A desktop app (egui/eframe; web later) for the
people who own the books and the accountant who checks them. Same
audience as QuickBooks, none of its visual language.

## What it is for

It answers the questions an owner actually has, then shows the evidence:

- **How much money do we have, and how long does it last?** Cash today,
  the trend, burn, runway, what's coming due.
- **Are we making money?** Revenue, spending, and profit by month; what
  changed and why.
- **Where does the money go?** Spending by category and vendor, over time.
- **Who owes whom?** Payables, receivables, cards, money owed to members.
- **Is anything wrong or missing?** Entries without receipts, drafts never
  posted, unreconciled accounts, uncategorized tax lines, 1099 vendors with
  no W-9, past periods left open, a journal that fails verification.
- **Can I trust this number?** Every number drills down to the lines that
  make it, each line to its entry, each entry to its rendered receipt and
  its complete journal history (who did what, when, hash-chained).

An accountant gets the same data at full depth: statements with interval
columns and comparisons, the general ledger, trial balance, the raw
journal with its verification status.

## Principles

1. **Answer first.** Every screen leads with the answer in words and one
   big number, then the chart, then the table, then the raw data.
2. **Everything drills down.** Any amount is a link. Navigation is a
   history stack (back/forward, like a browser) with a breadcrumb.
3. **All the data, nothing hidden.** Withdrawn attachments, reversals,
   drafts, every journal event: visible, marked for what they are.
4. **Read-only by construction.** The GUI has no write path: it reads the
   hash-chained journal and the blob store and builds its own in-memory
   view (`ledger_store::Snapshot`). Changes happen through the agent (MCP).
   Where the GUI finds something to fix, it offers a ready-to-paste prompt
   for the agent ("Attach the receipt for entry 12 — Anthropic, $7.40").
5. **Live.** It follows the journal: a change made by the agent appears
   within a second, highlighted as new.
6. **Numbers are typography.** Tabular figures everywhere (Inter with
   `tnum` frozen in), right-aligned, parentheses for negatives in
   statements, a minus sign elsewhere, never a float.
7. **No accounting logic in the GUI.** Statements, balances, and series
   come from `ledger-core` (new reports land there, where MCP can use
   them too). The GUI arranges and draws.

## Screens

- **Overview** — KPI tiles with sparkline and change vs. the prior period
  (cash, runway, revenue, spending, net income, owed by / owed to);
  cash over time; income vs. spending by month; spending by category;
  needs-attention list; recent activity from the journal.
- **Cash** — every cash and card account with balance and trend; inflows
  and outflows by month; burn and runway; known upcoming outflows
  (recurring entries, bills due, schedules).
- **Profit** — P&L by month as chart and table, spending breakdown with
  month-over-month change, by class.
- **Transactions** — the register of every entry: search, filters
  (account, vendor, class, date, status, receipt yes/no, amount), and a
  detail pane with lines, the rendered receipt (PDF pages, images),
  vendor, reversal links, and the entry's journal history.
- **Reports** — P&L, balance sheet, cash flow, trial balance, general
  ledger, members' equity, tax summary, A/R and A/P aging, 1099
  worksheet. Date presets, interval columns (total / month / quarter /
  year), comparison to the prior period or year. Every amount drills down.
- **Accounts** — the chart with balances and tax categories; an account
  opens its register with a running balance and a balance chart.
- **Documents** — every receipt and company document, rendered, with its
  category, entry link, and withdrawal status.
- **Company** — profile, members and capital, periods, fixed assets,
  amortization schedules, recurring entries, vendors and 1099 status.
- **Journal** — the raw event log, filterable, each event expandable to
  its JSON, with the hash-chain verification result.

Global: command palette (Ctrl+K) to jump to any account, entry, vendor,
document, or report; back/forward (Alt+←/→); light and dark themes.

## Architecture

- `ledger-store::Snapshot`: read-only. Verifies the journal chain,
  replays it into a `Ledger`, projects it into an in-memory SQLite with
  the same code the writer uses, reads blobs. `refresh()` applies only
  new journal lines (or reloads if the file was rewritten).
- `ledger-config` (moved out of `ledger-mcp`): `wanderbooks.toml`.
- `ledger-gui`:
  - `data` — a loader thread polls the journal and sends new snapshots.
  - `model` — view models built from a snapshot (pure, unit-tested).
  - `ui/*` — one module per screen; `widgets` (money, tiles, sparkline,
    tables, charts); `theme`; `receipts` (PDF via `hayro`, images via
    `image`; textures cached by blob hash).
  - Tests: model unit tests, `egui_kittest` screenshots of every screen
    against a fixture book.
- Web build later: the model is plain data, so a server can ship it.

## Status (2026-10-12)

Built: every screen above, the palette, history, themes, live updates
(tested end to end: a store write reaches the view model in ~30 ms),
receipt rendering for PDFs and images, a deterministic demo company
(`ledger-gui --demo`; also the screenshot fixture), and a screenshot test
(`SHOT_DIR=… cargo test -p ledger-gui --test screens`, optionally
`SHOT_BOOKS`, `SHOT_ONLY`, `SHOT_DARK`).

Next, in rough order: a budget-free cash forecast with a confidence band;
reconciliation view (cleared vs. uncleared per bank account); receipt
zoom and multi-page navigation; keyboard navigation in lists;
virtualized lists for very large books; the web build (ship the view
model from a server).
