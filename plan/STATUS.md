# STATUS — 2026-10-11 (all 10 natural next steps done; repo now public)

## Where we are

All ten natural next steps are done, tested, and pushed. The repo is now
public. Parity is 12 of 12 against the spreadsheet workflow.

**Done:**
- Repo `swaits/wanderbooks` on git.swaits.com (public as of 2026-10-11).
- `ledger-core` 0.1.0: `Money` (exact Decimal + ISO currency), five account
  types with normal sides, chart of accounts (codes, hierarchy,
  placeholders), journal entries with full validation, `Ledger` command
  surface (create/add-line/post/reverse), append-only posting log.
- Reports: trial balance, P&L, balance sheet (accounting equation holds;
  current earnings folded into equity pre-close), canonical JSON `snapshot()`
  for oracle diffing.
- 14 tests green: validation rejections, posted-entry immutability,
  reversal semantics, the accounting laws on a fixture company, a 300-entry
  randomized invariant test, and the first golden file.
- `ledger-store` 0.1.0: the source of truth is an append-only, hash-chained
  JSONL journal (fsync per append; tamper-evident); content-addressed
  SHA-256 blob store (immutable, deduplicated, hash-verified on read);
  SQLite as a *disposable* projection (TEXT amounts, WAL, `synchronous=FULL`,
  `integrity_check` on open). Single write path: validate → journal →
  mutate → apply. `verify` replays the journal and diffs reports three ways;
  `rebuild` drops SQLite and replays; a missing database heals on open.
  7 tests green: round-trip, tamper detection, torn-tail truncation,
  rebuild, blob dedup/corruption detection, unbalanced-post rejection.
- `ledger-mcp` 0.1.0: stdio server on official rmcp 3.5 (23 narrow typed
  tools, amounts as decimal strings, `initialize.instructions` carrying the
  agent operating contract). Draft → add lines → preview → confirmed post;
  server-side receipt policy (strictly greater than $25, per-account
  overrides, typed `receipt_required` rejection); digest; search; CSV/JSON
  export; period P&L and as-of balance sheet; fixed-asset tools (register,
  schedule, list, run-depreciation, dispose with preview). End-to-end stdio
  protocol tests green (initialize → tools → draft → preview →
  receipt_required → attach → post → reports → digest → reverse; and a full
  asset flow: register → schedule → idempotent runs → disposal preview →
  receipt_required → bill-of-sale → confirmed disposal → gain posted).
- **Fixed assets** (`ledger-core` + `ledger-store` + `ledger-mcp`):
  fixed-asset register (description, purchase date, cost, salvage, useful
  life, linked cost/accumulated-depreciation/expense accounts), straight-line
  book depreciation with day-prorated partial months and a final-month
  true-up so the lifetime total is exactly (cost − salvage), idempotent
  monthly runs, disposal with auto-posted final depreciation and gain/loss.
  Tax depreciation stays with the CPA. Tests: 10 asset unit tests
  (ledger-core), 6 store tests, 7 property tests, MCP e2e.
- **Testing initiative** (in progress): `proptest` property suite on
  `ledger-core` (trial balance always balances, A = L + E, reversals restore
  zero, posted-entry immutability, JSON round-trip, depreciation exactness —
  the last caught a real month-length-drift bug, fixed with the final-month
  true-up); `cargo-mutants` run on `ledger-core` (228 mutants); `cargo-fuzz`
  targets (journal-line parsing, entry validation, op sequences) scaffolded.
- 120+ tests green workspace-wide; `cargo fmt` clean; `cargo clippy
  --workspace --all-targets` clean, zero warnings (a hard rule: the owner,
  2026-10-11).
- Docs: AGENTS.md (operating manual), requirements, architecture, oracle
  design, parity catalog (floor: 10 done), glossary, mcp-interface (incl. the
  5 asset tools), receipt-store (journal design), capability-parity
  (10 of 12 rows covered). Plan: this file + roadmap.

**In progress:** nothing — the next arcs are queued below, unstarted.

## Next arcs (in order)

The project went AI-first on 2026-10-10 (the owner's call): the primary
interface is MCP, not a cloned GUI. The first storage + MCP slice is done
(arcs 1–3 below). What remains:

1. ~~**`ledger-store`**~~ done — journal + blob store + SQLite projection.
2. ~~**Blob store**~~ done — folded into `ledger-store`.
3. ~~**`ledger-mcp`**~~ done — 18 tools, receipt policy, digest, export.
   Remaining from the spec: MCP *resources* (rmcp 3.x has no resource
   macros; hand-roll `list_resources`/`read_resource` next), Streamable HTTP
   transport, XLSX/IIF export formats.
4. **Capability parity** (`docs/capability-parity.md`): 11 of 12 rows
   covered; full CPA export (xlsx/iif) remains. Then the month-long parallel
   run against the GSheet; cutover per the cutover rule.
5. Then: oracle harness (GnuCash diff), read-only GUI on the event log,
   workflow parity.

## Decisions log

- 2026-10-10: Single currency per ledger for day one (R-5). Multi-currency
  is designed-for, not built.
- 2026-10-10: `Money` arithmetic is checked and returns `Result`; no
  panicking ops, no float paths anywhere.
- 2026-10-10: Negative zero is normalized away at `account_balance` — money
  never prints "-0". (Caught by the golden test on day one; the oracle works.)
- 2026-10-10: License MIT OR Apache-2.0, matching the storytold projects.
  GnuCash clean-room boundary documented in `docs/oracle.md`.
- 2026-10-10: AI-first pivot (the owner). Primary interface is MCP
  (`docs/mcp-interface.md`), not a cloned GUI. Receipts/documents become a
  content-addressed blob store (`docs/receipt-store.md`). Parity target for
  the phase is the GSheet+GDrive system (`docs/capability-parity.md`), with
  a parallel-run cutover rule. GUI drops to read-only, event-log-driven,
  phase 4.
- 2026-10-10: Import/export targets QuickBooks formats (CSV, IIF, XLSX) —
  "could not give a single shit about GnuCash XML." GnuCash stays the
  *behavioral* oracle only. Receipt enforcement is server-side policy in the
  MCP layer (threshold + per-account overrides, typed `receipt_required`
  error, checklist in every preview); the engine stays policy-free.
- 2026-10-10: Storage durability (the owner). SQLite is the query engine, not
  the source of truth: the source of truth is an append-only, hash-chained
  JSONL journal (fsync per append; tamper-evident; git-backup-friendly),
  SQLite is a disposable projection rebuilt from it, blobs stay
  content-addressed. Single write path: journal → fsync → apply to SQLite.
- 2026-10-10: Draft durability (house decision). Drafts are **durable**, not
  ephemeral: `entry.created` + `entry.line_added` journal lines record the
  draft as the agent builds it, so drafts and their attached receipts survive
  restarts. Drafts never touch the SQLite projection until posted. Rationale:
  the journal is the source of truth (phantom `entry.created` lines without
  draft lines were the worst of both worlds), and the draft → attach receipts
  → post workflow spans multiple MCP calls.
- 2026-10-10: Corrupt SQLite heals itself. If the projection fails
  `integrity_check` (or isn't a database at all), `Store::open` quarantines
  it to `wanderbooks.corrupt.<timestamp>` and rebuilds from the journal,
  instead of failing open. WAL/SHM are dropped as derived state.
- 2026-10-10: Cash flow statement (R-FUTURE-3) done. Engine
  `reports::cash_flow` (indirect method) + `books.cash_flow` MCP tool (24
  tools total). Disposals identified structurally; gains/losses move from
  operating to investing; the three sections always reconcile to the net
  change in the designated cash accounts. Two invariant tests, including a
  full purchase → depreciate → dispose cycle.
- 2026-10-10: MCP resources done (hand-rolled `list_resources` /
  `read_resource`: chart, trial-balance, balance-sheet, profit-and-loss,
  digest, receipts). `#[tool_handler]` must sit on the `impl ServerHandler`
  block, not the inherent impl — misplacing it silently drops all tools.
- 2026-10-10: QuickBooks interop (R-FUTURE-4) done. New `ledger-import`
  crate: QBO bank CSV parse (3-col and 4-col layouts), chart CSV parse,
  IIF journal generation, XLSX report generation (rust_xlsxwriter, MIT).
  MCP: `books.import_bank_csv` (drafts + categorization rules),
  `books.import_chart_csv` (all-or-nothing), `books.export` gains `xlsx`
  (base64) and `iif` formats plus `cash_flow` report. 26 tools total.
  Capability parity 12 of 12 rows covered.
- 2026-10-10: Streamable HTTP transport done. `ledger-mcp --http ADDR`
  serves MCP at `/mcp` via rmcp's tower StreamableHttpService (axum).
  27 tools total. Verified with curl: initialize returns SSE stream.
- 2026-10-10: Drive backfill path done. `books.import_local_file` reads a
  file from the server's filesystem into the blob store (receipt/document).
  The agent downloads from Drive via the CLI, then calls this — Drive
  credentials never enter the server.
- 2026-10-11: `wanderbooks init` done. New `ledger-cli` crate (binary
  `wanderbooks`, std-only): `init [DIR]` scaffolds a books dir — `mise.toml`
  (pinned Rust toolchain; tasks: build, mcp-add, backup), `AGENT.md` (agent
  operating contract: bootstrap, MCP contract, first-run onboarding incl.
  migration via conversation, ongoing rules), `wanderbooks.toml` (company
  profile, filled during onboarding), `books-data/` + `.gitignore` (SQLite
  projection ignored; journal is the record). Fresh dirs get `git init` and
  a scaffold commit so the journal archive starts on day one. One generic
  init by design — no per-situation flags; the agent is the import layer
  (QBO/CSV/email/sheets, conversational). 5 integration tests green; clippy
  clean. Bootstrap: `mise trust && mise install`, `mise run build`,
  `mise run mcp-add`, then `claude` in the dir.
- 2026-10-11: Receipts are mutable through events, never edits (the owner:
  "the journal is just a history of every single thing that happens").
  `books.unlink_receipt` and `books.withdraw_attachment` (both require a
  reason) journal `receipt.unlinked` / `attachment.withdrawn`; nothing is
  erased from the journal or blob store. Unlinking can't strand an entry the
  receipt policy requires evidence for (attach the replacement first).
  `link_receipt` now refuses an already-linked or withdrawn attachment, which
  closes a silent path for moving a receipt off a posted entry; disposals
  check their receipt ids before posting. Schema v5. First real use: swapping
  the migrated the first company `.eml` receipts for their PDF renders.
- 2026-10-11: The cash flow statement is a first-class output everywhere the
  other two statements are: `cash_flow_pdf` in ledger-import, a `pdf`
  format on `books.export` (P&L, balance sheet, cash flow), and
  `cash-flow.pdf` in the CPA package, which now requires `cash_accounts`
  (the statement reconciles to them). Known gaps, next: the P&L and balance
  sheet PDF/XLSX ignore start/end/as_of (all-time balances under the
  requested label; core needs period-scoped statements); P&L and balance
  sheet XLSX are totals-only; PDF amounts don't align (padding in a
  proportional font), no company name, negatives as "-x".
- 2026-10-11: Every statement export (csv, json, xlsx, pdf, CPA package)
  now renders core's period statements: the date range and as-of date are
  real, with a line per account. One layout (`ledger-import::report`:
  sections, lines, subtotals, totals; exact "(1,234.56)" formatting) feeds
  every renderer. The server reads `wanderbooks.toml`: company name for
  report headers, `fiscal_year_start` for fiscal YTD (end derived), and
  `receipt_threshold` (it was silently ignored before). Reports carry
  "Prepared from journal through seq N".
- 2026-10-11: PDFs are typeset with Typst (typst 0.15 via typst-as-lib,
  fonts bundled by typst-assets; printpdf removed). One template,
  `ledger-import/templates/statement.typ`, takes the laid-out statement as
  JSON data (labels are never markup). US accounting presentation in PDF
  and XLSX alike: tabular figures, "$" on each section's first amount and
  on totals, negatives in parentheses with the closing paren hanging so
  decimal points align, zero as a dash, single rule over sums, double rule
  under totals, amounts never bold (bold figures are wider). Pages flow
  with a repeated header and "Page n of m". Core's cash flow now orders
  lines by account code (it followed HashMap order, so line order could
  change between runs).
- 2026-10-11: The canonical report set. Core gained the general ledger by
  account (running balances; QuickBooks' "split" = the opposite side of the
  entry, "-Split-" when several accounts), the trial balance as of a date,
  and the statement of changes in members' equity (capital roll-forward per
  member, allocation by share to the cent, ties to balance-sheet equity).
  The entry-ordered listing is now correctly called the general journal.
  Reports render QuickBooks-style (the owner's reference): Inter (bundled,
  OFL) instead of the typst-assets serif fonts, centered title block, a
  column-header row, multi-column statements (trial balance Debit/Credit,
  one column per member), "$" on sums, rules above sums, a double rule
  under totals, zero as 0.00, entity terms (Members' equity, Net income
  (loss), standard cash-flow wording) and an "Accrual basis · Prepared …"
  footer. "Today" is local time, not UTC (evening exports were dated
  tomorrow).
- 2026-10-11: Period close fixed and made partnership-aware. The close swept
  every revenue/expense posting ever made, so a period closed after later
  entries existed swallowed them (closing January in March closed February
  and March too); it now zeroes balances as of the period end, refuses to
  close out of order, and its preview is the real close run on a copy of
  the ledger. A multi-member LLC closes into each member's capital account
  by share (CloseTo::Members), the partnership convention, so capital
  accounts carry allocated earnings; the members'-equity statement treats
  those closes as income, not contributions, and the balance sheet shows
  only unallocated earnings. Replay now checks closing lines, not just ids.

