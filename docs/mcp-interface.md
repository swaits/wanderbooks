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
- `books.cash_flow { start?, end?, cash_accounts: ["1000"], financing_accounts? }`
  → indirect-method cash flow statement. `cash_accounts` are the reconciling
  accounts (required); the three sections always sum to their net change.
  Disposal gains/losses move from operating to investing automatically.
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
  structured, machine-readable error (JSON in the tool-error string):
  `{ "code": "receipt_required", "message": "…", "entry_id": 5,
  "debits": "100.00", "policy": "strictly greater than $25.00",
  "remedy_tool": "books.attach_receipt",
  "remedy_hint": "…" }`. Clients match on `code`, not prose. The agent's job
  then is to get the receipt attached (ask the human for it) and re-confirm
  — not to shrink the amount or split the entry to dodge the threshold. The
  tool description says this explicitly; gaming the policy is a correctness
  violation, and the append-only audit log would show it anyway.
- **Anti-splitting.** Posting an entry triggers a split check: posted
  entries on the same date with the same normalized description and debit
  accounts are grouped, and if the group total exceeds the threshold while
  the entry alone does not, a receipt is required. The preview surfaces
  `split_detected: [...]`, and the error names the group. Same-day,
  same-description, same-accounts is the signature; recurring spending
  spread across days is not flagged.
- **Below the threshold**, receipts are still expected per the operating
  contract ("a material entry without its source document is incomplete")
  but not hard-required — the preview checklist keeps them visible.

## Tools — export

- `books.export { format: "csv" | "json" | "xlsx" | "iif", report: "journal" |
  "trial_balance" | "profit_and_loss" | "balance_sheet" | "cash_flow" }` →
  `{ content, content_encoding? }`. CPA-ready output. `xlsx` is base64-encoded;
  `iif` is journal-only (QuickBooks Desktop). Cash flow needs `cash_accounts`.

## Tools — import

- `books.import_bank_csv { csv_text, bank_account_code, default_offset_code,
  rules? }` → creates one draft per transaction (QBO upload layout).
  `rules` maps description substrings to account codes.
- `books.import_chart_csv { csv_text }` → creates accounts from
  `code,name,type,parent_code` CSV. All-or-nothing.
- `books.import_local_file { path, category: "receipt" | "document", ... }`
  → imports a file from the server's filesystem. The Drive backfill path:
  the agent downloads from Google Drive via the CLI to a temp path, then
  calls this. Keeps Drive credentials out of the server.

## Transports

- **stdio** (default): `ledger-mcp --data-dir PATH`. For local agents.
- **Streamable HTTP**: `ledger-mcp --data-dir PATH --http 127.0.0.1:8080`.
  Serves MCP at `http://127.0.0.1:8080/mcp` via rmcp's tower integration.
  For network clients; bind to loopback unless you know what you're doing.

## Resources (read-only)

Hand-rolled on `ServerHandler` (rmcp 3.x has no resource macros):
`list_resources` advertises, `read_resource` renders.

- `books://chart`
- `books://reports/trial-balance`, `books://reports/balance-sheet`,
  `books://reports/profit-and-loss`
- `books://digest?since=2026-10-01`
- `books://receipts/{id}` — receipt metadata + base64 bytes

Resources exist so read-only clients (dashboards, the future read-only GUI,
auditors) never need tool credentials. Tested end-to-end over stdio.

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

## Tools — periods

- `books.create_period { name, start, end }` → `{ period_id }`.
  Dates are YYYY-MM-DD, inclusive.
- `books.close_period { period_id, confirmed? }` → without confirmation:
  `{ status: "preview", lines: [...] }` showing the closing lines and posts
  nothing; with `confirmed: true`: posts the closing entry (revenue debited
  to zero, expenses credited to zero, net to `3999` Retained Earnings) and
  marks the period closed. Closed periods reject new entries.
- `books.list_periods {}` → `{ periods: [{ id, name, start, end, status,
  closing_entry }] }`.

## Tools — bank reconciliation

- `books.mark_cleared { entry_id, line_index, cleared }` → `{ ok }`.
  Only posted lines can be marked; the flag is workflow metadata, not
  accounting data (posted entries stay immutable).
- `books.reconcile { account, statement_date, statement_balance }` →
  `{ account_code, book_balance, cleared_balance, statement_balance,
  difference, uncleared_lines: [{ entry_id, line_index, date, description,
  side, amount }] }`. `difference` is cleared − statement; zero means
  reconciled.

## Tools — A/R and A/P

- `books.add_customer { name }` → `{ id }`.
- `books.add_vendor { name }` → `{ id }`.
- `books.create_invoice { party_id, date, due_date, lines: [{ description,
  amount }] }` → `{ id, total, entry_id }`. Auto-posts Dr `1200` Accounts
  Receivable / Cr `4000` Revenue.
- `books.create_bill { party_id, date, due_date, lines }` → `{ id, total,
  entry_id }`. Auto-posts Dr `6999` General Expenses / Cr `2000` Accounts
  Payable.
- `books.receive_payment { document_id, date, amount, cash_account }` →
  `{ entry_id, remaining, status }`. Posts Dr Cash / Cr A/R.
- `books.pay_bill { document_id, date, amount, cash_account }` →
  `{ entry_id, remaining, status }`. Posts Dr A/P / Cr Cash.
- `books.ar_aging { as_of }` → `{ rows: [{ name, current, days_30, days_60,
  days_90, days_90_plus, total }] }`.
- `books.ap_aging { as_of }` → same, per vendor.

## Tools — foreign exchange

- `books.add_line` gains optional `foreign_currency`, `foreign_amount`,
  `fx_rate`. When set, the base amount is `foreign_amount * fx_rate`; the
  foreign original is stored on the line for reference. Reports work in
  base currency.
- `books.set_fx_rate { currency, date, rate }` → `{ ok }`. Records a
  historical rate for reference; lines store their own rate at entry time.
