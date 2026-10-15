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

## Company settings

`ledger-mcp` reads the books dir's `wanderbooks.toml` (default: next to
`--data-dir`; override with `--books-toml PATH`): `[company] name` heads
every report, `[company] fiscal_year_start` ("MM-DD", default "01-01";
the year's end is derived, never stored) defines fiscal-year-to-date for the
CPA package and the balance sheet's net income line, `[[company.members]]`
(`name`, `share` as "50%" or "0.5", `capital_account` code; shares must sum
to 100%) drives the statement of members' equity, `[books]
accounting_basis` ("accrual" or "cash", default accrual) labels report
footers, and `[books] receipt_threshold` sets the receipt policy unless
`config.json` sets `receipt_policy`. Members may name a
`distribution_account`; single-owner businesses may set `[books] close_into`
(the equity account a close puts net income in; default Retained Earnings)
and `close_sweep` (draw, distribution, or dividend accounts closed into it).
The cash flow statement's account roles are `[books] cash_accounts` (what
it reconciles to), `financing_accounts` (loans, and money the owners
advanced, such as a "Due to owners" liability: ASC 230-10-45-14(b)), and
`investing_accounts` (loans the company makes, investments it buys: ASC
230-10-20); the tools and the viewer both read them, and a call may pass
its own (docs/spec/rules.md B-14).

The server reloads `wanderbooks.toml` (and `config.json`'s receipt policy)
whenever either changes, compared by content at the start of each request,
so an edit takes effect on the next call with no restart. A broken edit
keeps the last good settings in force and is reported until fixed; the
data dir and base currency are fixed at startup (`ledger-mcp/src/settings.rs`).

- `books.settings {}` → `{ books_toml, books_toml_exists, config_json,
  reload_error, loads, company, fiscal_year_start, tax_form, basis,
  receipt_rule, members, cash_accounts, financing_accounts,
  investing_accounts, close_into, close_sweep, problems }`: the settings in
  force and where they come from. `problems` lists settings that disagree
  with the chart (a code not in it, or of the wrong kind), each saying what
  to change; `books.digest` repeats them, with a failed reload, as
  `settings_problems`. The agent edits the file when the human gives a
  company fact, a refusal names a setting, or a problem is listed, then
  checks `books.settings`.

The accounts invoices, bills, payroll, and closes post to are not settings:
they are control roles in the journal (rules.md B-23).

- `books.roles {}` → `{ roles: [{ role, purpose, account_type,
  default_code, assigned, account_code?, account_name? }] }`: every control
  role and the account filling it.
- `books.assign_role { role, account? }` → `{ roles }`: assign a role one
  of the chart's accounts (`account` null: back to the default code).
  Refuses the wrong type, a placeholder, an account already filling
  another role, and moving a balance sheet role while its account holds a
  balance. Journaled as `role.assigned`.

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
  amount: "123.45" }` → `{ ok: true }`. A refused line (unknown account or
  class, bad amount) adds nothing.
- `books.post_entry { entry_id, confirmed }` → `{ status, entry_id, date,
  description, total, lines (with resulting balances), receipts,
  receipt_required, receipt_missing, split_detected }`. Without
  `confirmed: true`, `status` is `"preview"` and nothing changes; with it,
  the receipt policy is enforced and `status` is `"posted"`.
- `books.reverse_entry { entry_id, date, description, confirmed }` → same
  preview/confirm pattern as post.
- `books.get_entry { entry_id }` → full entry with lines.
- `books.search_entries { account?, date_from?, date_to?, text?, limit?, offset? }`
  → entries with lines, newest first. `limit` caps at 100.
- `books.trial_balance { as_of? }` → `as_of`, lines, totals, `balanced`
  (the trial balance as of the date, default today).
- `books.balance_sheet { as_of? }` → totals (`assets`, `liabilities`,
  `equity`, `current_earnings`, `balanced`) plus `asset_accounts`,
  `liability_accounts`, `equity_accounts` (`{code, name, amount}`), as of
  the date (default today).
- `books.profit_and_loss { start?, end?, class? }` → totals plus
  `revenue_accounts` and `expense_accounts` for the range. `end` defaults to
  today, `start` to the first posted entry. Closing entries are excluded, so
  a closed period still shows what it earned.
- `books.cash_flow { start?, end?, cash_accounts?, financing_accounts?, investing_accounts? }` (each defaults to `[books]` in wanderbooks.toml; cash is required one way or the other)
  → indirect-method cash flow statement. `cash_accounts` are the reconciling
  accounts (required); the three sections always sum to their net change.
  Disposal gains/losses move from operating to investing automatically.
- `books.digest { since: "2026-10-01" }`
  → `{ entries_posted: [...], totals_by_account: [...], stale_rules: [...] }`.
  The human review surface: "what did the AI do with my books lately." The
  digest is the answer to "trust, but verify." `stale_rules` lists legal
  amounts wanderbooks may no longer know as of today (`{ name,
  known_through, catalog, source, why }`): research the current value or
  ask the human, then update it upstream (pull request or issue at
  github.com/swaits/wanderbooks).

## Tools — fixed assets

Straight-line **book** depreciation only; tax depreciation stays with the
CPA. Partial first/final months prorated by days; the final month trues up
so the lifetime total is exactly (cost − salvage).

- `books.register_asset { description, purchase_date, cost, salvage_value?,
  useful_life_months, cost_account, accum_depr_account, expense_account,
  tag?, serial? }` → `{ asset_id, tag }`. Accounts resolved by id or code.
  Without `tag`, the next free `PREFIX-0001` (`[books] asset_tag_prefix`,
  else the company name's initials without LLC/Inc; "FA" when none).
- `books.label_asset { asset, tag?, serial?, clear_serial? }` → `{ asset_id,
  tag, serial }`: set or change the tag (unique ignoring case, never reused
  by another asset even after a retag or disposal) or the manufacturer's
  serial. `asset` is the id or current tag. Journaled as `asset.tagged`,
  `asset.serial_set`.
- `books.asset_labels { assets?, skip? }` → `{ file_name, content_base64,
  tags }`: an Avery 5160/8160 label sheet (PDF): company, tag, QR code of
  the tag, description, serial. Default: every active tagged asset; `skip`
  reuses a partly used sheet.
- `books.asset_schedule { asset_id | tag }`
  → `{ status, schedule: [{ period, amount, accumulated, book_value }] }`.
- `books.list_assets {}` → assets with status and current book values.
- `books.run_depreciation { period: "YYYY-MM", asset_id? }`
  → per-asset `{ entry_id, amount, already_posted }`, plus `skipped` for
  assets outside their useful life. Idempotent per asset and period.
- `books.dispose_asset { asset_id | tag, date, proceeds, proceeds_account,
  gain_loss_account, confirmed?, receipt_attachment_ids? }` → without
  confirmation: `{ status: "preview", ... }` and posts nothing; with
  `confirmed: true`: auto-posts missing final-period depreciation, then
  posts the balanced disposal entry. Subject to receipt policy (bill of
  sale); monthly depreciation is exempt as an allocation.
- `books.export { report: "fixed_asset_register", format: json | csv,
  as_of? }`: every asset registered by the date (tag, serial, cost,
  depreciation by schedule, book value, disposal), totals, and the
  general ledger's balances of the assets' cost and accumulated
  depreciation accounts, with `unposted_depreciation` and
  `unregistered_cost` (zero when register and books agree;
  `ledger_core::fixed_asset_register`).
- `books.show { asset }` opens an asset in the viewer; `page: "assets"`
  and `report: "fixed_asset_register"` open the register.

## Tools — schedules and recurring entries

Amortization spreads a prepaid expense (asset → expense) or deferred
revenue (liability → revenue) evenly over the months it covers, posting on
each month end, to the cent (the last month absorbs rounding). Like
depreciation, the schedule is the evidence: its postings are exempt from
the receipt policy (the original payment carries the receipt). Recurring
entries never post: they generate drafts, so every real receipt still goes
through the policy.

- `books.create_schedule { kind: "prepaid_expense" | "deferred_revenue",
  description, total, start, months, balance_account, pl_account }` →
  `{ schedule_id, ..., total, remaining, months: [{ date, amount, posted }] }`.
  Prepaid needs an asset and an expense account; deferred revenue a
  liability and a revenue account.
- `books.list_schedules {}` → `{ schedules: [...] }`, same shape.
- `books.run_schedules { through?, confirmed? }` → `{ status: "preview" |
  "posted", postings: [{ schedule_id, date, description, amount,
  entry_id? }] }`: every unposted month ending on or before `through`
  (default today). Idempotent; closed periods are refused.
- `books.create_recurring { description, frequency: "weekly" | "monthly" |
  "quarterly" | "yearly", start, end?, lines: [{ account, side, amount }] }`
  → `{ recurring_id, ..., generated: [] }`. Lines must balance. Monthly and
  longer occurrences are computed from the start date each time, so a
  month-end start stays on month ends (Jan 31, Feb 28, Mar 31).
- `books.list_recurring {}` → `{ recurring: [...] }`.
- `books.run_recurring { through? }` → `{ drafts: [{ recurring_id, date,
  entry_id }] }`: one draft per occurrence on or before `through`, once.

## Tools — the viewer

- `books.show { entry_id | account | report | attachment_id | search |
  page, start?, end? }` → `{ shown, note }`. Asks the human's viewer
  (`ledger-gui`) to open an entry, an account register or a report (for
  start/end dates if given), a stored file, a transactions search, or a
  page. Exactly one target; entries, accounts, and attachments are checked
  to exist. Never touches the books: the request is a small JSON file in
  the user's runtime directory (`$XDG_RUNTIME_DIR/wanderbooks/viewer-<hash
  of the data dir>.json`), written atomically, watched by the viewer. A
  viewer started within two minutes of a request opens it; older requests
  are ignored.

## Tools — integrity

- `books.verify { blobs? }` → `{ status: "ok" | "healed", events, head,
  healed?, blobs_verified?, checked }`. Verifies the journal's hash chain,
  replays it against the live ledger, and compares every table of the
  SQLite database row for row against a fresh projection of the journal.
  A drifted database is rebuilt from the journal (the old file kept as
  `wanderbooks.mismatch.<time>`). With `blobs` (default true), re-hashes
  every stored file. The server runs the same check, and heals the same
  way, every time it starts.

## Tools — payouts

- `books.record_payout { date, description, deposit_account, deposit,
  revenue: [{ account, amount, class? }], fees?: [{ account, amount,
  class? }], adjustments?: [{ account, side, amount, class? }] }` →
  `{ entry_id, status: "draft", gross, fees, adjustments, deposit, lines,
  next }`. Builds the grossed-up entry — Dr deposit (net; Cr when negative),
  Dr fees, Dr/Cr adjustments (a debit reduces the deposit: refunds,
  chargebacks, reserves held, tax withheld), Cr revenue (gross; revenue or
  liability accounts, e.g. deferred revenue or sales tax collected) — and
  refuses unless gross − fees ± adjustments equals the deposit to the cent,
  naming the difference. A draft: attach the payout statement, preview,
  post. Platform-agnostic; format-specific statement parsers can feed it.

## Tools — 1099 tracking

Payments are counted as cash: credits minus debits to asset accounts on
entries tagged with the vendor. Card payments move no asset (the card
processor's 1099-K covers them), so they drop out; reversals of tagged
entries carry the tag and net out. Thresholds: $600 for payments through
2025, $2,000 from 2026 (indexed for inflation after 2026); always
overridable, always for the CPA to confirm. TINs are never stored, only
whether a W-9 is on file.

- `books.list_vendors {}` → `{ vendors: [{ id, name, form_1099?,
  tin_on_file }] }`.
- `books.set_vendor_1099 { vendor, form_1099: "nec" | "misc" | "none",
  tin_on_file? }` → the vendor. `vendor` is an id or a name.
- `books.set_entry_vendor { entry_id, vendor? }` → the entry (as
  `books.get_entry`, which now shows `vendor`). Omitting `vendor` clears
  the tag. Metadata only: posted entries may be tagged. `books.draft_entry`
  takes `vendor` too; bills and their payments are tagged automatically.
- `books.vendor_1099 { year, threshold? }` → `{ year, threshold, vendors:
  [{ vendor_id, name, form_1099?, tin_on_file, paid, reportable }],
  missing_w9, notes }`. For a year whose threshold wanderbooks doesn't know
  (after 2026 until the indexed amount is added) it refuses with what to
  do; pass `threshold` explicitly once the figure is confirmed.

## Tools — receipts & documents

- `books.attach_receipt { entry_id?, file_name, mime_type, content_base64 }`
  → `{ receipt_id, sha256, deduplicated, size_bytes }`. `entry_id` is
  optional: a receipt can be filed before its entry exists.
- `books.link_receipt { receipt_id, entry_id }` → link a previously filed
  (unlinked, not withdrawn) receipt to its entry. A receipt linked elsewhere
  must be unlinked first.
- `books.unlink_receipt { attachment_id, reason }` → `{ ok, entry_id,
  remaining_receipts }`. Detaches a receipt from its entry; it stays filed,
  unlinked. Journaled as `receipt.unlinked` with the reason. Subject to the
  receipt policy: an entry that requires a receipt cannot be left with none
  (`receipt_required` error, remedy `books.attach_receipt`), so replacing a
  receipt is attach-new, then unlink-old.
- `books.withdraw_attachment { attachment_id, reason }` → `{ ok }`. Retires
  an unlinked receipt or a filed document: hidden from listings, never
  linkable again. Journaled as `attachment.withdrawn`; the bytes stay in the
  blob store. Nothing in the evidence trail is ever erased — receipts change
  by new events, the same way entries change by reversals.
- `books.list_receipts { entry_id?, include_withdrawn? }` → metadata list
  (no bytes), each with `withdrawn` / `withdrawn_reason`. Withdrawn
  attachments are omitted unless `include_withdrawn` is true.
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

- `books.export { format, report, start?, end?, as_of?, cash_accounts?,
  financing_accounts?, investing_accounts? }` → `{ content, content_encoding? }`.
  - Reports: `profit_and_loss`, `balance_sheet`, `cash_flow` (needs
    `cash_accounts`), `trial_balance`, `members_equity` (statement of
    changes in members' equity; members from `wanderbooks.toml`),
    `general_ledger` (by account: beginning balance, postings with running
    balance and split, ending balance), `general_journal` (alias
    `journal`: every entry, line by line, in date order).
  - Formats: `pdf` and `xlsx` (base64) for the five statements; `xlsx`,
    `csv`, `json` for every report; `iif` for the journal (QuickBooks
    Desktop).
  - Dates: period reports take `start` (default: first entry) and `end`
    (default: today, local time); `balance_sheet` and `trial_balance`
    take `as_of`.
  - Presentation (PDF and XLSX alike, QuickBooks-style): centered company,
    title, and period; a column-header row; "$" on subtotals and totals;
    negatives in parentheses with decimal points aligned; a rule above every
    sum and a double rule under statement totals; a footer with the
    accounting basis, preparation date, and journal seq. Multi-member LLCs
    get "Members' equity" with members named; the balance sheet separates
    retained earnings from the fiscal year's net income (loss).

## Tools — chart templates

- `books.chart_template { entity_type, owners? }` → `{ csv, settings_toml,
  notes }`: a starter chart for a sole proprietorship (incl. single-member
  LLC), partnership (incl. multi-member LLC), S corporation, or C
  corporation — the equity and owner-pay accounts that entity needs (owner's
  capital and draws; per-partner capital and distributions plus guaranteed
  payments; officer compensation and shareholder distributions; dividends
  and income tax expense), common income and expense accounts with tax
  categories, and the `wanderbooks.toml` settings (tax form, members, close
  target). A proposal: review with the human, then `books.import_chart_csv`.

## Tools — tax categories

- `books.tax_categories {}` → `{ form?, categories: [{ key, label, kind,
  default_deductible_percent, note }] }`: where an account can land on the
  income tax return, filtered to the business's return when known
  (`[company] tax_form`, or derived from `tax_classification`). Stable
  semantic keys (`meals`, `guaranteed_payments`, `commissions_fees`, ...),
  never form line numbers, which change by year.
- `books.renumber_account { account, code }` → `{ id, previous_code,
  code, config_mentions }`: give an account a new code. Only the code
  changes (entries name accounts by id), so balances and reports don't
  move. Refuses a code in use, or an unassigned control role's default
  code for an account of the wrong type. `config_mentions` lists the
  wanderbooks.toml settings still naming the old code (edit them; the
  server reloads the file). Journaled as `account.renumbered`.
- `books.set_tax_category { account, category?, deductible_percent? }`:
  map a revenue or expense account to a category (null clears),
  optionally overriding its deductible percent. Journaled as
  `account.tax_set`.
- `books.export { report: "tax_summary" }`: book vs. tax amounts by
  category, the difference where they disagree (meals 50%, penalties 0%),
  and every P&L account still uncategorized. In the CPA package for the
  fiscal year to date.
- `books.import_chart_csv` accepts an optional fifth `tax_category` column,
  validated before anything is created.

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
  `{ status: "preview", lines: [...] }`: the exact closing entry (the close
  run on a copy of the ledger), posting nothing; with `confirmed: true`:
  posts it and marks the period closed. The entry zeroes revenue and
  expense balances as of the period end; the net goes to `3999` Retained
  Earnings, or, with `[[company.members]]` in `wanderbooks.toml`, to each
  member's capital account by share. Periods close in order. Closed periods
  reject new entries.
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

## Tools — payroll (accounting only)

No tax tables: the caller supplies every number; the engine cuts correct
entries.

- `books.run_payroll { date, cash_account, lines: [{ employee_name,
  gross_pay, federal_withholding, state_withholding, ss_employee,
  medicare_employee, ss_employer, medicare_employer, other_deductions }] }`
  → `{ run_id, entry_id, total_gross, total_net }`. One balanced entry:
  Dr 6100 Salary & Wage Expense (gross), Dr 6110 Payroll Tax Expense
  (employer SS + Medicare), Cr 2110–2150 withholding payables, Cr cash
  (net). Auto-creates the 61xx/21xx accounts. Respects closed periods.
- `books.remit_payroll_taxes { date, cash_account }` → `{ entry_id, total }`.
  Clears every payroll payable to cash in one entry.
- `books.list_payroll_runs` → `{ runs: [{ id, date, employee_count,
  total_gross, entry_id }] }`.

## Tools — classes (divisional tracking)

- `books.add_class { name }` → `{ id }`. E.g. "Rondo", "Coach vs Coach".
- `books.list_classes` → `{ classes: [{ id, name }] }`.
- `books.add_line` gains optional `class` (name or id): tags the new line.
  Shared costs are split explicitly across lines at entry time.
- `books.profit_and_loss` gains optional `class` filter.
- `books.profit_and_loss_by_class { start, end, class? }` → per-class revenue,
  expenses, net income, plus an unclassified row (only that class's row
  when `class` is given; an unknown or ambiguous class is refused). Reversals carry their
  class so divisional P&L nets correctly.

## Tools — CPA handoff

- `books.export_cpa_package { as_of?, cash_accounts?, financing_accounts?, investing_accounts? }`
  → `{ file_name, content_base64 }`. A zip with the full set, each
  statement as PDF and XLSX: `profit-and-loss`, `balance-sheet`,
  `cash-flow` (reconciled to `cash_accounts`), `members-equity`
  (multi-member LLCs), `trial-balance`; plus `general-ledger.xlsx` and
  `general-journal.xlsx`. Period reports cover the fiscal year to date.
  What the CPA needs; they never log into the system.
