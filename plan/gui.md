# ledger-gui — design

Phase 4 of the roadmap (`plan/roadmap.md`). A desktop app (egui/eframe) for the
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
  - `data` — a loader thread follows the journal (notify-driven, polling as the fallback) and sends new snapshots.
  - `model` — view models built from a snapshot (pure, unit-tested).
  - `ui/*` — one module per screen; `widgets` (money, tiles, sparkline,
    tables, charts); `theme`; `receipts` (PDF via `hayro`, images via
    `image`; textures cached by blob hash).
  - Tests: model unit tests, `egui_kittest` screenshots of every screen
    against a fixture book.
- Web build later: the model is plain data, so a server can ship it.

## Status (2026-10-28)

Built 2026-10-12 (the first version): every screen above, the palette,
history, themes, live updates (a store write reaches the view model in
~30 ms), receipt rendering for PDFs and images, a deterministic demo
company (`wanderbooks gui --demo`; also the screenshot fixture), and a
screenshot test (`SHOT_DIR=… cargo test -p ledger-gui --test screens`,
optionally `SHOT_BOOKS`, `SHOT_ONLY`, `SHOT_DARK`).

Built since, by the "Served" plan (`plan/served.md`):

- **The data root is the only source** (D-44): `wanderbooks gui <book>`
  opens a book of the data root, read-only, its settings from the
  journal and its vault from its own `vault/` and `keys/`; `--data-dir`
  and `--books-toml` are gone. The viewer holds the books open (a lease
  of kind app) and still opens with the server stopped.
- **The Books menu** (P2 slice 8, D-19): Back up now, Restore…, New
  books…, Connect Claude Code, Connect Codex, Close the books, each
  handed to the server through the same functions the command line
  uses; the Export menu (the CPA package, the 1099 worksheet, Save a
  copy, Save the W-9), each a new file in Downloads. The viewer writes
  nothing of the books (`tests/read_only.rs`, `tests/owner_menu.rs`).
- **The format-upgrade dialog** (D-39): asked at every open of waiting
  books, a standing notice, the owner passphrase typed in the viewer.
- **Color schemes** (D-19's menu): the viewer's own family and twenty
  editor families, 80 schemes, each at WCAG AA contrast, worn live on
  hover (`tests/schemes.rs`, `docs/color-schemes.md`).
- **Needs attention is the engine's** (U0b, D-53): one function in
  `ledger-package::attention` over `PackageBooks`, read by the viewer and
  by `books.start`; the viewer only maps each item's target to a route,
  and `the_viewer_and_books_start_see_the_same_attention` proves the two
  lists equal. Principle 4's "ready-to-paste prompt" now comes from that
  engine.
- **Pages added:** Invoices & Bills (Money in, Money out, a document's
  page with its timeline, overview tiles; P4b S2) and Save invoice PDF
  (S3); Projects and a project's snapshot (D-43); parties with roles,
  W-9 and the TIN clipboard copy (D-29); fixed assets with the tax-basis
  figures (P3); reconciliation; the journal page describing every event
  in plain words.
- **Compatibility tests:** `tests/old_books.rs` (books an earlier version
  wrote open to their figures), `tests/newer_journal.rs` (a newer journal
  says "update wanderbooks", never half-read), `tests/view_model.rs`
  (every figure shown is the engine's).

Landed 2026-10-29: the invoice page shows sends (S5), a discount-deadline
column and the tie-out badge (S6), write-offs on the timeline (S7),
documents grouped by category and subject (P6). S4.5 adds the number,
terms, bill-to and the issued copy to Save.

Landed 2026-10-29 (P11b): **the agent dock** along the bottom of the window
(Claude Code or Codex in the book's books home, over a PTY with
`alacritty_terminal`; collapsible and resizable; the choice remembered;
Linux checked, macOS by CI, Windows not offered until its ConPTY checks are
recorded; `dock.rs`, `tests/dock.rs`) and **the first-run page** (no books yet:
the private folder, the owner passphrase, the first books, an agent, then
the books open; `welcome.rs`, `tests/welcome.rs`), which is what lets an
installed app onboard with no terminal. Both go through the `Owner`; the
viewer still writes nothing of the books.

Next, in the plan's order: S4.5's page changes; P4's draft states in
Needs attention; P11a (the installed app: the viewer is its face, with
"Check for updates" in the menu; P11b, the dock and the first run, is
landed, above). Not planned: a web build, which belongs to the hosted
service (`plan/served.md` section 11, a browser viewer); the remote
viewer replica (D-51: a backup clone opened read-only serves instead).
Still open from the first version's list: a cash forecast with a
confidence band, receipt zoom and multi-page navigation, keyboard
navigation in lists, virtualized lists for very large books.
