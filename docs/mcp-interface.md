# MCP interface spec — `ledger-mcp`

## Goal

The primary interface to the books is an AI agent driving MCP tools. The
engine (`ledger-core`) stays pure, `ledger-store` persists, and `ledger-mcp`
exposes both over the Model Context Protocol — the open standard, so the
books are drivable by any MCP client (Muse, Claude Code, IDE agents), not
locked to one assistant.

## Crates in this phase

| Crate | Role | Spec |
|---|---|---|
| `ledger-store` | SQLite persistence, blob metadata, event log | `docs/receipt-store.md` |
| `ledger-mcp` | MCP server wrapping store + core | this document |

Rust, `rmcp` SDK (the Rust MCP SDK). The MCP crate holds no accounting
logic: it translates tool calls into `Ledger` commands and formats results.
If a behavior can't be expressed through `ledger-core`'s public API, the
core is missing something — fix the core, don't work around it in the MCP
layer.

## Transports

1. **stdio** (build first). For Claude Code, Claude Desktop, local agents.
   No auth — the OS user boundary is the auth.
2. **Streamable HTTP** (later in the phase). For remote access (phone, travel
   laptop). Bearer token, binds 127.0.0.1 or the tailnet interface only.
   Binding 0.0.0.0 is an explicit human decision, never a default.

## Tool design principles

- **Narrow tools, explicit arguments.** No "do what I mean" tools. The agent
  supplies dates, accounts, sides, amounts; the server validates.
- **Amounts are decimal strings** in requests and responses. Never floats,
  never locale-formatted numbers.
- **Every mutating tool echoes the exact resulting state.** The "show the
  numbers" rule from AGENTS.md applies over the wire too.
- **Preview-before-post.** `post_entry` and `reverse_entry` take
  `confirmed: bool`. Without confirmation they return a preview (full lines
  + resulting account balances) and post nothing. The agent must present the
  preview to the human before confirming. This encodes the standing
  fact-check rule in the protocol itself.
- **Errors are typed strings** mirroring `LedgerError` variants, with the
  offending values included (e.g. unbalanced totals).

## Tools — books

- `books.chart` → `{ accounts: [{ id, code, name, type, parent, placeholder,
  balance }] }`. Balances included: one call answers "what do we have."
- `books.draft_entry { date: "2026-10-10", description: "..." }`
  → `{ entry_id, status: "draft" }`.
- `books.add_line { entry_id, account: "1000" | <id>, side: "debit" | "credit",
  amount: "123.45" }`
  → `{ lines: [...], totals: { debits, credits, balanced } }`. Running totals
  after every line: the agent watches the entry balance in real time.
- `books.post_entry { entry_id, confirmed }` → without confirmation:
  `{ preview: { lines, resulting_balances } }`; with confirmation: validates,
  posts, returns `{ entry, balances_changed: [{ code, before, after }] }`.
- `books.reverse_entry { entry_id, date, description, confirmed }` → same
  preview/confirm pattern as post.
- `books.get_entry { entry_id }` → full entry with lines.
- `books.search_entries { account?, date_from?, date_to?, text?, limit?, offset? }`
  → entries with lines, newest first. `limit` caps at 100.
- `books.trial_balance {}` → canonical JSON (same shape as
  `reports::snapshot` trial-balance section).
- `books.balance_sheet {}` → canonical JSON.
- `books.profit_and_loss {}` → canonical JSON.
- `books.digest { since: "2026-10-01" }`
  → `{ entries_posted: [...], totals_by_account: [...] }`. The human review
  surface: "what did the AI do with my books lately." The digest is the
  answer to "trust, but verify."

## Tools — receipts & documents

- `books.attach_receipt { entry_id?, file_name, mime_type, content_base64 }`
  → `{ receipt_id, sha256, deduplicated, size_bytes }`. `entry_id` is
  optional: a receipt can be filed before its entry exists.
- `books.link_receipt { receipt_id, entry_id }` → link a previously filed
  receipt to its entry.
- `books.list_receipts { entry_id? }` → metadata list (no bytes).
- `books.get_receipt { receipt_id }` → `{ metadata, content_base64 }`.
- `books.file_document { category: "formation" | "legal" | "tax" |
  "accounting" | "other", file_name, mime_type, content_base64,
  description? }` → `{ document_id, sha256, deduplicated }`. The Drive-folder
  replacement (today's 01 Formation / 02 Legal / 03 Tax / 04 Accounting).

## Tools — export

- `books.export { format: "csv" | "json", scope: "journal" | "trial-balance" |
  "profit-and-loss" | "balance-sheet" | "chart" }` → `{ content_base64,
  file_name }`. CPA-ready output. GnuCash XML arrives with R-FUTURE-4.

## Resources (read-only)

- `books://chart`
- `books://reports/trial-balance`, `books://reports/balance-sheet`,
  `books://reports/profit-and-loss`
- `books://digest?since=2026-10-01`
- `books://receipts/{id}` — receipt bytes with metadata

Resources exist so read-only clients (dashboards, the future read-only GUI,
auditors) never need tool credentials.

## The agent operating contract

Any AI client driving these tools follows this contract (it will also live
in the server's tool descriptions, so the model reads it at use time):

1. **Show the preview before confirming any posting.** No silent posts, ever.
2. **Echo exact numbers back to the human.** Amounts as written, no rounding
   in prose.
3. **Attach receipts at record time.** A material entry without its source
   document is an incomplete entry. File first, link later if the entry
   isn't ready.
4. **When unsure of categorization, draft and ask.** A draft is cheap; a
   wrong posting is a reversal. Never guess-post.
5. **The digest is the receipt for your work.** After a batch of bookkeeping,
   offer the digest; the human reviews, you fix.

## Out of scope for this phase

Bank feeds, payment processing, payroll, tax filing, multi-user authN/Z, the
GUI (it consumes the event log in a later phase). Cash flow reporting is
R-FUTURE-3 and rides the engine roadmap, not the MCP layer.
