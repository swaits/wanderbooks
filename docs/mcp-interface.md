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

## Server instructions

The MCP `initialize` response carries an `instructions` string: the agent
operating contract (below) in condensed form. The model reads it at the start
of every session, before any tool call. Tool descriptions repeat the relevant
fragment at the point of use (`post_entry`'s description states the
preview-before-post rule; `attach_receipt`'s states the receipt rule). Prose
guidance is therefore delivered twice — once at session start, once at the
tool — but it is guidance, not enforcement. Enforcement is the receipt policy
below.

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
  `{ preview: { lines, resulting_balances, receipts: { attached: [...],
  required_but_missing: bool } } }`; with confirmation: validates (including
  the receipt policy), posts, returns `{ entry, balances_changed: [{ code,
  before, after }] }`.
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

## Tools — fixed assets

Straight-line **book** depreciation only; tax depreciation stays with the
CPA. Partial first/final months prorated by days; the final month trues up
so the lifetime total is exactly (cost − salvage).

- `books.register_asset { description, purchase_date, cost, salvage_value?,
  useful_life_months, cost_account, accum_depr_account, expense_account }`
  → `{ asset_id }`. Accounts resolved by id or code.
- `books.asset_schedule { asset_id }`
  → `{ status, schedule: [{ period, amount, accumulated, book_value }] }`.
- `books.list_assets {}` → assets with status and current book values.
- `books.run_depreciation { period: "YYYY-MM", asset_id? }`
  → per-asset `{ entry_id, amount, already_posted }`, plus `skipped` for
  assets outside their useful life. Idempotent per asset and period.
- `books.dispose_asset { asset_id, date, proceeds, proceeds_account,
  gain_loss_account, confirmed?, receipt_attachment_ids? }` → without
  confirmation: `{ status: "preview", ... }` and posts nothing; with
  `confirmed: true`: auto-posts missing final-period depreciation, then
  posts the balanced disposal entry. Subject to receipt policy (bill of
  sale); monthly depreciation is exempt as an allocation.

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

## Receipt enforcement

Prose tells the model to attach receipts; the policy makes it true. The
engine stays policy-free (it knows nothing of receipts) — enforcement lives
in the MCP layer, which reads the `attachments` table before confirming a
posting.

- **Policy config** (server config file, not per-call): `require_receipts_above`
  (default `"25.00"` in base currency), plus `receipt_required_accounts`
  (account codes that always require receipts, e.g. meals and travel) and
  `receipt_exempt_accounts` (e.g. automated bank fees). Human-edited and
  versioned with the deployment.
- **Preview.** Every `post_entry` / `reverse_entry` preview includes the
  receipt checklist (`attached: [...]`, `required_but_missing: bool`), so the
  human sees it before confirming.
- **Hard error.** Confirming a posting that violates the policy fails with a
  typed error: `{ "error": "receipt_required", "entry_id": …, "missing": …,
  "policy": "require_receipts_above = 25.00 USD" }`. The agent's job then is
  to get the receipt attached (ask the human for it) and re-confirm — not to
  shrink the amount or split the entry to dodge the threshold. The tool
  description says this explicitly; gaming the policy is a correctness
  violation, and the append-only audit log would show it anyway.
- **Below the threshold**, receipts are still expected per the operating
  contract ("a material entry without its source document is incomplete")
  but not hard-required — the preview checklist keeps them visible.

## Tools — export

- `books.export { format: "csv" | "json", scope: "journal" | "trial-balance" |
  "profit-and-loss" | "balance-sheet" | "chart" }` → `{ content_base64,
  file_name }`. CPA-ready output. `xlsx` and `iif` (QuickBooks Desktop
  journal entries) arrive with R-FUTURE-4 via the `ledger-export` crate.

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
   isn't ready. Postings at or above the receipt policy threshold *cannot*
   post without one — the server refuses with `receipt_required`.
4. **When unsure of categorization, draft and ask.** A draft is cheap; a
   wrong posting is a reversal. Never guess-post.
5. **The digest is the receipt for your work.** After a batch of bookkeeping,
   offer the digest; the human reviews, you fix.

## Out of scope for this phase

Bank feeds, payment processing, payroll, tax filing, multi-user authN/Z, the
GUI (it consumes the event log in a later phase). Cash flow reporting is
R-FUTURE-3 and rides the engine roadmap, not the MCP layer.
