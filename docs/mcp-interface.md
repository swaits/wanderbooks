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

The books keep their own settings (journal format 2, plan/served.md P1):
each change is a `settings.set` line with who decided it and when, replayed
into `ledger_core::settings::Settings` by the server and the viewer alike
(rules.md B-29). Every key is in `ledger_core::settings::KEYS`, with what it
takes and means: the company's facts (`company.name`, `.entity_type`,
`.tax_classification`, `.tax_form`, `.fiscal_year_start` ("MM-01"; the
year's end is derived, never stored), `.timezone` (what "today" means for
the books), `.formation_state`, `.formed`, `.state_entity_number`, `.ein`
(the one identity number a setting keeps, shown masked: "**-***5893"),
`.irs_name_control`, `.duns`, `.address`, `.registered_agent`,
`.partnership_representative`, `.cpa`, `.members` (name, share as "50%" or
"0.5", capital account, optional distributions account and
`files_jointly_with`; shares sum to exactly 1)); the books'
(`books.accounting_basis`, `.cash_accounts`, `.financing_accounts` (loans,
and money the owners advanced: ASC 230-10-45-14(b)), `.investing_accounts`
(ASC 230-10-20), `.close_into` (single owner only), `.close_sweep`,
`.asset_tag_prefix`, `.schedule_convention` (daily, the default, or
monthly_even)); backups (`backup.git_remote`, `backup.folder`); and policy (`policy.evidence.threshold` (receipts
strictly above it; "0.00": every entry), `.required_accounts`,
`.exempt_accounts`, `policy.parties` (off, suggest, require), `policy.require_tax_category`
(warn, the default, or refuse),
`policy.approval_rules` (off, the default, or enforce)). A value is checked
by its key and kept in one spelling (a time zone as the database spells it,
an amount without leading zeros, a return as `schedule_c`, `1065`, `1120s`
or `1120`); a change is all or nothing, and the settings after it must hold
together (members and `close_into` never both, a classification that names
one return, a fiscal year that doesn't move under a closed period). No
setting holds nine digits that read as an SSN or EIN (bare, or printed
3-2-4 or 2-7) except `company.ein` and `company.duns`.

Books from before P1 hold no `settings.set` and still read the books dir's
`wanderbooks.toml` (default: next to `--data-dir`; `--books-toml PATH`)
and `config.json`'s `receipt_policy`, reloaded whenever either changes, as
before. `books.import_settings` moves them in once; from then on no file
is read, not even at start (a broken file beside the books can't stop the
server). A vault key file is never a books setting: `--vault-key-file PATH`
names it to the server and the viewer (it wins over a file's `[vault]
key_file`).

- `books.settings {}` → `{ source ("books" or "file"), timezone, today,
  approval_rules, keys: [{ key, doc, identity, value, set_by, seq }],
  books_toml, books_toml_exists, config_json_present, reload_error, loads,
  company, fiscal_year_start, tax_form, basis, receipt_rule,
  receipt_required_accounts, receipt_exempt_accounts, parties, members,
  cash_accounts, financing_accounts, investing_accounts, close_into,
  close_sweep, problems }`: the settings in force, every key the books know
  with its value (null: unset, its default applies) and who set it, and
  `problems`, settings that disagree with the chart, each saying what to
  change (conventions naming an account no longer in the chart among
  them); `books.digest` repeats them as `settings_problems`. Settings are
  named by key for books that keep their own, as the file spells them for
  books that still read one; for those, `keys` shows the file's values,
  with no journal line.
- `books.set_setting { changes: [{ key, value }], decided_by, decided_on?,
  citation?, note? }` → `{ changed: [{ key, was, value }], problems }`: one
  or more changes, together or not at all (`value` null clears a key back to
  its default; a value already set changes nothing, idempotent;
  destructive, since it can clear or switch off a setting). Refused while
  the books still read a settings file (or `config.json`): import it first,
  so no setting is lost.
- `books.import_settings { toml_text, config_json_text?, confirmed?,
  skip_unknown?, decided_by, decided_on?, citation?, note? }` → `{ status
  ("preview" or "imported"), changes, notes, unknown, comments }`: exactly
  the changes
  that make the books' settings what the file (and `config.json`'s
  `receipt_policy`, which wins as it does for the server) says. `notes` are
  what isn't a books setting and where it goes (`[books] data_dir`, `[vault]
  key_file`, `[viewer]`, `base_currency`); `unknown` are entries the books
  don't know, not imported (confirm with `skip_unknown`); `comments` are
  the file's comments, in order, which are not settings and are not
  imported (record what matters in them as a decision, open item,
  convention or a setting's note). The text must be
  the file the server reads, whole (its import is compared with the
  server's own reading of its files, key by key), and the server must read
  every setting afterwards as it does now; the vault key must then come
  from `--vault-key-file` (on the viewer too). A file never turns approval
  rules on. Books that keep their own settings take no import that changes
  them (an identical one is no change). Without `confirmed`, a preview that
  records nothing. An identity value exported masked imports as
  "unchanged".
- `books.export_settings {}` → `{ toml_text }`: the settings as a settings
  file, each with who decided it as a comment, the EIN masked; a copy, and
  importing it back changes nothing (`ledger_config::codec`).

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

## Tools — how the books are kept

Conventions, decisions, open items and approvals live in the books too
(`ledger_core::conventions`, rules.md B-29), each with who decided it and
when; `books.start` lists them, and the handbook's
settings-and-conventions section says how to use them.

- `books.start { focus? }` (read-only, at most 8 KB) → `{ book: { name,
  timezone, today, fiscal_year, base_currency, basis }, head: { seq, hash },
  state ("needs_onboarding" or "active"), settings_source, rules: {
  evidence, parties, approval_rules }, settings_summary: [{ key, value,
  set_by, seq }], conventions: [{ id, kind, title, summary }], open_items:
  [{ id, title, due }], projects: [{ id, name, status, kind? }],
  truncated?, handbook_sections, format_upgrade?: {
  guard, format, adds, how }, next_steps: [{ tool, args?, why }],
  isolation }`. The connect text names it as the first call.
  `format_upgrade` is there only for books waiting for a format upgrade
  (below); its next step comes first, before anything else, at every
  start until it is done: `books.handbook {section: "format-upgrade"}`,
  "Tell the human first: …".
  `isolation` is `same_user (advisory)` (D-32): the books are hidden from
  agents in the owner's own account, not locked against them. Lists cut
  to fit say so in `truncated` ("conventions: 4 more (books.conventions)").
  `projects` lists the active and paused projects by name, at most 20
  ("projects: N more (books.projects)" past that).
  `next_steps`, in order: the format upgrade's (when waiting);
  `books.import_settings` (books that still read a settings file); the
  onboarding step (`books.handbook {section: "contract"}`, when the chart
  is empty or `focus` is `onboarding`); `books.open_items` (an item due
  within 30 days or overdue); `books.backup` (none set up, or one stale,
  failing, paused or failing its drill); the focus's step (`close`:
  `books.conventions {kind: "close_step"}`; `audit`: `books.verify`); then
  always `books.handbook {section: "contract"}` and `books.digest`.
  `focus` is `daily` (the default), `onboarding`, `close` or `audit`;
  anything else is refused. Not built yet (P5):
  `book.id`, `book.url`, `head.last_handoff`, the `agent` input, and
  loading it up front under tool search. Also `books://start`.
**The format upgrade** (plan 3.9, D-39, rules.md B-27). Books an older
wanderbooks wrote are in an older journal format: they are served on it,
and every tool that needs nothing new works as before. A tool whose
change needs the newer format (on main: settings, conventions, decisions,
open items and approvals need journal format 2; later formats add
features) is refused before anything is journaled, its error beginning
`format_upgrade_pending:` and saying that only the owner can give the
go-ahead, with the owner passphrase, in the app or with `wanderbooks
books upgrade ID` at a terminal, and that no tool can. No tool consents:
none is offered, and the server takes the go-ahead only on its admin
socket, from the host's lease intent `consent_format_upgrade` (the app's
dialog, `books upgrade`), with the passphrase it checks. Before the
upgrade the server commits the books to their own backup repository and
proves the copy holds them exactly (journal, stored files, vault, keys);
then one `journal.format_set` declares the whole format, and the same
session's next call can make the change. Books this version creates are
in its format from their second line, and an adoption's copy moves to it
as it is made.

- `books.conventions { kind?, include_retired? }` → every convention with
  its full text, structure, revision and source (`books://conventions`).
  `books.set_convention { id, kind, title, body_md, structured?,
  decided_by, ... }` sets or revises one (kinds: `posting_template`,
  structured `{ description?, lines: [{ account, side, share }] }`, each
  side's shares summing to 1, on accounts in the chart that take postings
  or on slots (`"{expense}"`) each draft names;
  `approval_rule`, structured `{ approver, accounts?, amount_over? }`; and
  `policy`, `close_step`, `calendar`, `mapping`, `note`, in words). An id is
  a slug, revised in place (same kind), never reused once retired:
  `books.retire_convention { id, reason, decided_by, ... }` (destructive).
- `books.draft_from_template { template_id, date, amount, accounts?,
  description?, party?, no_party_reason? }` → `{ entry_id, description,
  lines }`: a draft whose lines split the amount (at most a line's) by the
  template's shares, each side summing to it exactly, odd cents by largest
  remainder; `accounts` names each slot's account (`{"expense": "6300"}`),
  every slot and nothing else. A refused draft journals nothing (a disk
  failing midway could leave a partial draft, as for any draft built line
  by line).
- `books.record_decision { subject, decision_md, decided_by: [..],
  decided_on, evidence_attachment?, supersedes?, citation?, projects? }` →
  `{ id }`; `books.decisions { include_superseded?, project? }`. A decision
  supersedes at most one live decision; its evidence is a document still
  in the books. `projects` (ids or names, each once, resolved before
  anything is written) names the projects it is about (rules.md B-37); a
  decision without them journals the bytes it always did, and one with
  them needs the journal feature `projects`. `books.decisions` gives each
  decision's `projects: [{ id, name }]` (left out when none); `project`
  filters to the decisions naming one.
- `books.raise_item { title, detail_md?, due?, decided_by, ... }` → `{ id }`,
  `books.resolve_item { id, resolution, decided_by, ... }`,
  `books.open_items { include_resolved? }`.
- `books.approve_entry { entry_id, approver, note?, approved_on? }`: the
  approver approved the draft as it stands (its fingerprint: date,
  description, every line and class; a line added or a class changed
  afterwards voids it). Under `policy.approval_rules = enforce`,
  `books.post_entry`'s preview lists `approvals_missing` and a confirmed
  post is refused with `approval_required` until each covering rule's
  approver approved (the store refuses it too), and a tool that posts at
  once (an invoice, a bill, a payment, payroll, a remittance, a disposal)
  is refused when a rule covers the entry it would post: draft it instead.
  Reversals, closes, depreciation and amortization are exempt. A rule
  names accounts that take postings; an account a rule names can't be
  renumbered until the rule is revised, and `books.renumber_account` lists
  the conventions that name the old code. Under `off` (the default) rules
  block nothing.

## Transports

- **stdio** (default): `ledger-mcp --data-dir PATH`. For Claude Code,
  Claude Desktop and other local agents. No auth: the OS user boundary is
  the only boundary.
- **Streamable HTTP**: `ledger-mcp --data-dir PATH --http 127.0.0.1:8080`.
  Serves MCP at `http://127.0.0.1:8080/mcp` via rmcp's tower integration.
  **There is no authentication on this transport today**: no token and
  no Origin check. The only guard is rmcp's default `Host` check, which
  accepts `localhost`, `127.0.0.1` and `::1`. Any local process that can
  reach the port can read and change the books. Bind it to loopback only;
  binding a LAN, tailnet or public interface is an explicit human
  decision, never a default. Auth, scopes and Origin validation arrive
  with the served books (`plan/served.md`, P2).
- **Served books** (`wanderbooks server`, started by the host; an agent
  connects with `wanderbooks mcp --book ID`, a stdio bridge): every book
  in the data root, on a socket in the root's private `run/` folder that
  only this account can connect to. Every request carries a bearer token
  live for that book (refused 401 without one, 403 with another book's).
  Refusals at the door, tool calls, uploads, downloads and exports are
  recorded in the book's access log; when a record can't be written the
  book stops taking requests (an upload, download or export is then
  refused, 503).
  - `/b/{book}/mcp`: the book's MCP service. A request is at most 1 MiB.
  - `PUT /b/{book}/u/{upload}`: an upload's bytes (`books.begin_upload`),
    from the token that began it; what `wanderbooks upload --book B
    [--token NAME] UPLOAD FILE` sends. Streamed to the disk, kept only if
    exactly the size and SHA-256 declared (so at most 25 MiB), once. The
    receive and its access record are one blocking task: bytes are never
    kept without their record (if it can't be written they are thrown
    away, 503 `not-recorded`, and the books stop). 200 with `{ received,
    size, sha256 }`; else, with nothing kept and the code in
    `x-wanderbooks-refused`: 404 `no_such_upload` (unknown or lapsed,
    also if it lapsed while being sent), 403 `not_yours`, 409
    `already_sent` (its bytes came, or a call holds them), 409 `in_use`
    (another request is sending them), 400 `not_as_declared` (short, more than declared, or
    the wrong hash), 503 `stopped` (the books stopped after an internal
    error), 500 `failed`. A refused request's body is read and dropped
    (up to 25 MiB), so the client gets the answer, not a cut connection.
    Logged as action `upload` with the upload's id, a refusal at the
    door (no token, another book's) too. An upload's id is `up_` and 32
    letters `a`-`p`: no digits, so the access log's TIN masking never
    hides it.
  - `GET /b/{book}/a/{attachment}`: a stored file's bytes, for a token
    with `books:read` (403 `insufficient_scope` without it; 404
    `no_such_attachment`); the read and its access record are one
    blocking task. Sent with `Content-Type` (the stored MIME type, or
    `application/octet-stream` if it can't be a header),
    `Content-Disposition: attachment; filename="…"; filename*=UTF-8''…`
    (the plain name: anything but visible ASCII and spaces, and `"` and
    `\`, made `_`; then the whole name, RFC 8187 percent-encoded),
    `X-Content-Type-Options: nosniff`, and `Repr-Digest:
    sha-256=:<base64>:` (RFC 9530). A stored file that can't be read
    whole is 500 `failed`; books that stopped, 503 `stopped`. GET only
    (a HEAD is 405, unrecorded). Logged as
    action `download` with the attachment's id, a refusal at the door
    too.
  - `GET /b/{book}/x/{export}`: a file the books made for an agent (the
    CPA package, a 1099 worksheet, a W-9: a `HandedFile`, "Tools — the
    TIN vault"), as `/a/` (the same door, the same headers,
    `Repr-Digest` included, GET only), but only for the token it was made
    for (403 `not_yours`), with no scope of its own to check (the tool
    that made it checked its scope); 404 `no_such_export` (unknown or
    lapsed), 503 `stopped`. An export's id is `ex_` and 32 letters
    `a`-`p`; it is kept 15 minutes, fetched as often as wanted until
    then, at most 16 per token, and swept like uploads. Logged as action
    `export` with the export's id.
  - `wanderbooks download --book B [--token NAME] ID --to DIR` takes an
    attachment's id (a number, `/a/`) or an export's (`ex_…`, `/x/`),
    checks the digest before saving, and saves it under the
    server's name (`filename*` first) made plain: separators and control
    characters made `_`, invisible (Unicode format) characters and
    leading dots and spaces dropped, at most 200 bytes with the extension
    kept. Beside a file of that name it is `name (1).ext`, and so on;
    `attachment-<id>` or `export-<id>` if the folder takes no such name.
    It is written to a
    hidden file and hard-linked in: never a partial file under the real
    name, never over another file.

## Server instructions

The MCP `initialize` response carries an `instructions` string, the
**connect text** (`ledger-mcp/src/handbook.rs`), composed in `get_info`
from the settings in force when the client connects (the settings are
re-read first, so a client connecting after an edit is told the new rule).
It is short on purpose: clients cut long instructions (one cut the old
text at 4,096 characters), so it is at most **1,400 characters**, and its
first paragraph, at most **512**, stands alone: the entry point
(`books.start` first each session, and `books.handbook` for every
procedure), the rule that these tools are the only way to the books (never
local files), decimal-string amounts, and the draft → lines → preview →
confirmed post flow. The second paragraph adds
that every entry balances and uses real accounts from `books.chart`; the
parties rule in force (`[books] parties`: under "require" every entry
the agent posts names its party or a no-party reason, since closes,
depreciation and amortization post with none; under "suggest" name it
where there is one; under "off" nothing); corrections by reversal; the receipt rule in force
(`ReceiptPolicy::rule`: the exact threshold in the books' base currency,
never rounded, the accounts that always need a receipt and those exempt,
and that reversals need none); the exemptions (depreciation and
amortization, whose schedules are the evidence, and closes, derived from
the books); that the invoice, bill, payment and payroll tools post at
once, unchecked (in those words, naming no tool); TINs
only into the vault; show the numbers, and after posting check
`books.digest` and offer it to the human; and where settings live: in the
books, changed only through the tools, or, for books that still read a
settings file, that file, the one local file the agent may edit until
`books.import_settings` moves it in. When a long rule (many
accounts, a huge threshold, a long name) would push it past 1,400
characters, it drops the tracing hint first (the handbook's
tracing-figures section has it), then the reasons in parentheses, then the
tracing hint for good and lists fewer account codes (at last only how many
there are; `books.settings` lists every one, uncut, in
`receipt_required_accounts` and `receipt_exempt_accounts`), then words
that only help; never the rule itself. A company with no name yet (the
settings' default) is "a new company". The company name in it is one line of plain text (control characters and
line breaks become spaces, Unicode format characters such as bidi
overrides and zero-width spaces are dropped), cut to 56 characters.
`instructions_budget` (the longest rule, every parties setting) and
`the_company_name_is_one_plain_line` check it.

The long guidance is the **handbook**, compiled in from
`ledger-mcp/handbook/*.md`, with the receipt rule in force rendered in:

- `books.handbook { section? }` (read-only) → `{ read_first, sections:
  [{ id, title, uri, chars }], section: { id, title, text } | null }`.
  Without `section`, the index (`read_first` says to read `contract`
  first); with a section id, that section's Markdown; with `"all"`, the
  whole handbook in reading order. The input schema lists the ids:
  contract, entries, drafts-and-reversals, evidence-and-receipts,
  attachments, files-in-out, reconciliation, fixed-assets,
  prepaid-and-recurring, revenue-and-payouts, parties-and-tins, 1099,
  cpa-handoff, settings-and-conventions, viewer, backups, format-upgrade,
  rules-that-change, control-roles, tracing-figures.
- The same as resources: `books://handbook` (the index, JSON) and
  `books://handbook/{section}` (Markdown, `text/markdown`), one listed per
  section.

No text the agent is given (tool JSON with its schemas, resources, connect
text, handbook, and every refusal fixed in code: each error variant's
template in the crates under the server, scanned from source, built error
texts, and the server's refusal constants) names a helper command, a home
path, the data dir or its flag, the old import-root flag, a file of the books
(`.jsonl`, the database, `vault.key`, `.gitignore`), git, GitHub, a pull
request, a repo doc (`docs/`, `.md`), or tells the agent to read the
books' files; no tool title or description, connect text or handbook
section names `wanderbooks.toml` (the input and output schemas name it by
design, for `books.settings`' `books_toml`) (`tool_text_lint`). A rule set by law that the books
don't know for a year names its official source (the statute or
publication, with its address) and says to tell the human.
Tool descriptions are at most 700 characters; eight longer ones are listed
in `tool_descriptions_fit_700_characters`, to be shortened in P5.

**Annotations.** Every tool has a `title` and a `readOnlyHint`; every tool
that changes anything also has a `destructiveHint`, true for these 29:
`books.post_entry`, `books.reverse_entry`, `books.import_settings`,
`books.set_setting`, `books.retire_convention`, `books.unlink_receipt`,
`books.unlink_attachment`, `books.withdraw_attachment`,
`books.correct_asset`, `books.dispose_asset`, `books.run_depreciation`,
`books.run_schedules`, `books.run_recurring`, `books.renumber_account`,
`books.rename_account` (the label every past report prints),
`books.assign_role`, `books.close_period`, `books.reconcile`,
`books.void_statement`, `books.void_reconciliation`, `books.set_w9`,
`books.set_tin` (both overwrite a vault record), `books.purge_tin`,
`books.create_invoice`, `books.create_bill`, `books.receive_payment`,
`books.pay_bill`, `books.run_payroll`, `books.remit_payroll_taxes`; and an
`idempotentHint` on the eleven that a repeat with the same arguments leaves
the journal and the vault as they were: `books.verify`,
`books.run_depreciation`, `books.run_schedules`, `books.run_recurring`,
`books.set_party_roles`, `books.set_setting`, `books.import_settings`,
`books.set_vendor_1099`, `books.set_entry_instrument`,
`books.update_project` and
`books.back_up_now` (a setter such as `books.update_party`,
`books.set_fx_rate` or `books.set_tin` journals a line, and `books.set_tin`
seals a new vault record, every time, so none carries it;
`idempotent_tools_change_nothing_when_repeated` calls each candidate twice
to check). Every tool's
`openWorldHint` is false: the books are a closed world, and no tool
reaches beyond them, but `books.back_up_now`, which pushes the books to
the owner's backups (true). `books.export_cpa_package`, `books.vendor_1099`
and `books.get_w9` keep a file for the agent (an export) and record the
ones with TINs, so they are not read-only. No tool carries
`requiresUserInteraction` (owner decision
2026-10-16: no added prompts). `annotations_complete` pins all of this.
The read-only hint is true in behaviour: `spec_mcp` checks, on every call,
that a read-only tool leaves the journal, the contents of every file under
the data dir (the database by every row of every table; SQLite's -wal and
-shm files, which move pages and hold read marks on their own, aside),
the vault and the settings file as they were
(`read_only_tools_change_nothing`). `books.show` is read-only though it
writes the viewer's request into the books' `ops/` folder: that folder
is a side channel, never journaled, and moves only the view.

**Failing closed.** Tool bodies run on blocking threads, one at a time on
the store. If a request panics (an internal error), the journal may hold a
line the memory doesn't, so the server stops taking requests: every later
call, on every session sharing the store, is refused with "the books
server stopped taking requests after an internal error, to protect the
books: ask the human to restart the books server", never the panic's own
text. A restart re-verifies the journal
(`a_request_that_panics_stops_the_server`). So a panic an input can reach
would be a denial of service, and none can be: the dates a tool takes
are four-digit years and real days, its amounts at most a journal
line's, and a recurring run bounded; books an earlier version wrote past
those bounds still open (replay accepts what that version accepted), so
every figure the server derives from them (a draft's total, a schedule's
plan, aging, a party's totals, the asset register) is computed or
refused ("too large to compute exactly", or a date outside the calendar
the books keep), never a panic (rules.md B-28). `tests/hostile_args.rs`
drives every tool with generated hostile arguments and every resource
with generated URIs, checks after each call that the server still
serves, restarts it after them to check the books reopen, and drives
every tool over every book in ledger-store's `tests/fixtures/old-books`.

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

- `books.handbook { section? }`: the guide (see "Server instructions").
- `books.chart` → `{ accounts: [{ id, code, name, account_type, parent,
  placeholder, balance, tax_category, deductible_percent }], base_currency
  }`. Balances included: one call answers "what do we have."
- `books.add_account { code, name, account_type, parent?, placeholder?,
  tax_category?, deductible_percent? }` → `{ account_id, warnings? }`.
  Codes are unique; a name another account under the same parent has
  (top-level accounts are siblings too; ignoring capitals and surrounding
  space), one holding a run of nine digits, or one that isn't one line of
  at most 200 characters, is refused (rules.md B-31); the name is kept
  trimmed. A revenue or expense account (not a header) without `tax_category` is
  created with a warning ("6150 Software has no tax category: the tax
  summary will list it as uncategorized. Propose one from
  books.tax_categories and set it with books.set_tax_category once the
  human agrees."), or, under `policy.require_tax_category = refuse`,
  refused (rules.md B-32). The category is checked against the account's
  type and the return, as `books.set_tax_category` checks it; journaled as
  `account.added` then `account.tax_set`.
- `books.draft_entry { date: "2026-10-10", description: "...", vendor?,
  party?, parties?, no_party_reason? }` → `{ entry_id, status: "draft" }`.
  `party` tags the counterparty (see "Tools — parties"); given with
  `vendor`, both must name the same vendor record, else nothing is
  drafted. `parties` names several, each with a share (as
  `books.set_entry_party`), in place of `party` and `vendor`.
- `books.add_line { entry_id, account: "1000" | <id>, side: "debit" | "credit",
  amount: "123.45" }` → `{ ok: true }`. A refused line (unknown account or
  class, bad amount) adds nothing.
- `books.post_entry { entry_id, confirmed?, expected_total? }` → `{ status, entry_id, date,
  description, total, lines (with resulting balances), receipts,
  receipt_required, receipt_missing, split_detected, party_policy,
  party_id?, party?, parties, parties_error?, no_party_reason?,
  party_missing, party_suggestions, approvals_missing? }`. `expected_total`,
  when given, must be the entry's total debits, else nothing posts.
  Without `confirmed: true`, `status`
  is `"preview"` and nothing changes; with it, the receipt policy is
  enforced and `status` is `"posted"`. `parties` lists every party the
  entry names with its share resolved to the cent (`{ party_id, party,
  share: "amount" | "percent" | "derived", amount?, percent?, resolved }`);
  when the shares don't add up to what the entry moved, `parties_error`
  says why and confirming fails with code `party_shares`.
- `books.reverse_entry { entry_id, date, description, confirmed }` → same
  preview/confirm pattern as post. A reversed depreciation run or schedule
  month is un-posted (its month is due again); refused, at the preview
  already (P3): an asset's disposal entry (it would stay disposed), a
  depreciation run of a disposed asset (its disposal took it as final),
  and a reversal whose reversal would post a depreciation period or a
  schedule month a second time (naming the entry that posts it now), or
  re-instate a run a later run caught up (carrying the books past the
  schedule); and a reversal of a depreciation run or a schedule month's
  posting (or of a reversal in its chain) dated before that entry's own
  date (a correction comes after what it corrects, B-9).
- `books.get_entry { entry_id }` → full entry with lines, receipts, and
  its `vendor`, `party_id`, and `party` (the party's label, when it names
  exactly one) tags, `parties` / `parties_error` as in the `post_entry`
  preview, and `no_party_reason` when it is marked as having no
  counterparty. `total` (its debits) is null, with `total_error` saying
  why, for a draft an earlier version journaled whose lines exact
  arithmetic can't sum; a draft line on an account the chart doesn't have
  (older books hold such drafts) shows an empty code and "(no account N)".
- `books.search_entries { account_id?, date_from?, date_to?, text?, limit?,
  offset? }` → `{ entries: [{ entry_id, date, description, total,
  reversal_of, receipt_count }] }`, posted entries, newest first (no
  lines: `books.get_entry` has them). `limit` defaults to 50 and caps at
  500.
- **Omitted dates (rules.md B-30).** One rule everywhere
  (`ledger_core::calendar::PeriodDefaults`), stated in every report
  tool's description and date parameters in the same words: a period
  report with no dates covers the fiscal year to date
  (`company.fiscal_year_start`): `end` defaults to today (the company's
  time zone), `start` to the first day of the fiscal year that holds
  `end`; a `start` after the `end`, given or derived (`start` after today
  with no `end`), is refused, never swapped. An `as_of` defaults to
  today; a party's `year` to this calendar year. Every result names the
  dates it covers.
- `books.trial_balance { as_of? }` → `as_of`, lines, totals, `balanced`
  (the trial balance as of the date, default today).
- `books.balance_sheet { as_of? }` → totals (`assets`, `liabilities`,
  `equity`, `current_earnings`, `balanced`) plus `asset_accounts`,
  `liability_accounts`, `equity_accounts` (`{code, name, amount}`), as of
  the date (default today).
- `books.profit_and_loss { start?, end?, class? }` → totals plus
  `revenue_accounts` and `expense_accounts` for the range (omitted dates:
  the fiscal year to date). Closing entries are excluded, so a closed
  period still shows what it earned.
- `books.cash_flow { start?, end?, cash_accounts?, financing_accounts?, investing_accounts? }` (each defaults to the books' settings, `books.cash_accounts` and so on; cash is required one way or the other)
  → the statement of cash flows, direct method (rules.md B-14, P3 defect
  1; omitted dates: the fiscal year to date): `{ method: "direct", start,
  end, operating_by_class, operating, cash_from_operations,
  investing_by_kind, investing, cash_from_investing,
  financing_by_kind, financing, cash_from_financing, net_change_in_cash,
  cash_beginning, cash_ending, noncash_disclosures,
  operating_reconciliation, reconciles, warnings }`.
  - Lines (`operating`, `investing`, `financing`) are one per entry, section
    and kind: `{ entry, date, label, kind, amount, on_behalf?, accounts,
    reverses?, class? }`; every transaction is listed (D-35), each under its
    kind. An entry's interest is its own operating class, (b) or (e), apart
    from the rest of its operating cash. What a member or a lender paid or
    received for the company with no cash account moving is cash borrowed
    (or contributed) and spent, on the entry's date (D-42, rules.md B-14
    rule 11): an expense a member paid personally through Due to Members
    is a "Proceeds from borrowing — <account>" line and an operating
    line, each with `on_behalf`, the part of its amount so handled (absent
    when none; an entry's `on_behalf` parts net to zero).
    Labels never come from a reversal's sign: "Reversal of #N — …",
    "Reinstatement of #A (reverses #B)". An entry and its reversal in the
    same period drop out together.
  - `operating_by_class`, `investing_by_kind`, `financing_by_kind`:
    `{ kind, class?, amount, entries }`, how the printed statement shows the
    sections: operating by ASC 230-10-45-25 class (`from_customers`,
    `interest_and_dividends_received`, `other_receipts`,
    `to_suppliers_and_employees`, `interest_paid`, `income_taxes_paid`,
    `other_payments`), investing and financing by kind ("Contributions from
    members — <member>", "Repayments of borrowing — <account>", "Purchases
    of property and equipment", …).
  - `noncash_disclosures`: `{ entry, date, label, amount, operating,
    investing, financing, exchanged, reverses? }`, the investing and
    financing activities no cash settled (ASC 230-10-50-3): property
    contributed, debt converted to equity, a purchase on the seller's
    credit, interest charged into a loan, the noncash part of a part-cash
    purchase.
  - `operating_reconciliation`: `{ net_income, lines: [{ kind, label,
    amount, account?, entries }], net_cash_from_operating }`, net income
    reconciled to operating cash (ASC 230-10-45-30), kinds `depreciation`,
    `investing_gain_loss`, `noncash_through`, `other_noncash`,
    `operating_balance_change`.
  - `reconciles`: `{ sections_sum, net_change, difference }`; `difference`
    is always `"0.00"`: a statement that wouldn't foot is an engine error,
    never output.
  - `warnings`: `{ entry, kind, message }` for an entry whose later payment
    or collection the books can't classify (`asset_on_operating_payable`,
    `asset_sold_on_operating_receivable`), with the fix.
  - Refused, naming the setting, when the roles contradict the chart or
    each other (cash and investing accounts must be assets, financing
    accounts liabilities or equity; no account in two lists or twice in
    one; no registered asset's account as cash).
  - Changed in P3 (breaking): `net_income`, `depreciation` and
    `working_capital` are gone (net income is in `operating_reconciliation`).
- `books.digest { since: "2026-10-01" }`
  → `{ since, entries: [...], totals_by_account: [...], stale_rules: [...],
  journal_repairs: [...], settings_problems: [...], unreconciled_accounts:
  [...], uncategorized_accounts: [...] }`.
  The human review surface: "what did the AI do with my books lately." The
  digest is the answer to "trust, but verify." `stale_rules` lists legal
  amounts wanderbooks may no longer know as of today (`{ name,
  known_through, source, why }`): research the current value or
  ask the human, then update it upstream (pull request or issue at
  github.com/swaits/wanderbooks). `unreconciled_accounts` lists the cash
  and card accounts due for reconciliation (`{ code, name,
  reconciled_through?, due_through, next_step }`): the cash accounts
  (`[books] cash_accounts`, else asset accounts named for cash: cash,
  bank, checking, savings, money market), the card accounts (the
  business-card payment methods' liabilities, else liabilities named
  "card", not "gift", before their first statement:
  `ledger_core::parties::card_accounts`), and every other liability
  with a recorded statement, that have posted activity through the end of
  last month but no reconciliation (not voided) through it. The viewer's
  Needs attention uses the same rule (`ledger_store::reconcilable`).
  `uncategorized_accounts` lists every revenue and expense account (not a
  header) with no tax category, active or not, `"CODE Name"`: the list
  Needs attention shows (rules.md B-32).

## Tools — fixed assets

Straight-line **book** depreciation only; tax depreciation stays with the
CPA. Depreciation accrues by the day (rules.md B-15): a partial first
month, the life's final month (which takes what the others leave, evenly
over its days), and any month a schedule stops in (through a date, or a
disposed asset's disposal day, held) are prorated by days, never plugged;
the lifetime total is exactly (cost − salvage), rounded to the currency's
minor unit. A new asset's cost and salvage are in the minor unit, and its
life's last day is 9999-12-31 at the latest.

- `books.register_asset { description, in_service_date, cost,
  salvage_value?, useful_life_months, cost_account, accum_depr_account,
  expense_account, tag?, serial?, acquisition_entry?, basis_rule?,
  contributor_adjusted_basis?, fmv_at_contribution?, sec704c_method?,
  basis_source? }` → `{ asset_id, tag, in_service_date, acquired_date,
  basis?, notes? }`. Accounts resolved by id or code. Without `tag`, the
  next free `PREFIX-0001` (`[books] asset_tag_prefix`, else the company
  name's initials without LLC/Inc; "FA" when none). `in_service_date` (the
  day it is ready and available for its use) starts depreciation (P3,
  defect 13; B-15); `purchase_date` is its alias for one release (both
  given and meaning different days: refused; the alias noted in `notes`). The
  `acquisition_entry` (posted, debiting the cost account, neither a
  reversal nor reversed, dated on or before the in-service date) gives
  `acquired_date`, which puts the asset
  on the register; with none, the acquired date is the in-service date.
  The basis facts are `books.set_asset_tax_basis`'s (`tax_basis_rule` and
  `contributor_basis`, with the old rule names, are aliases for one
  release); a bad one registers nothing, and so do basis facts given
  without `basis_rule`.
- `books.label_asset { asset, tag?, serial?, clear_serial?, description?,
  acquisition_entry? }` → `{ asset_id, tag, serial, acquisition_entry,
  description }`: set or change the tag (unique ignoring case, never reused
  by another asset even after a retag or disposal), the manufacturer's
  serial, the description, or the linked acquisition entry (posted,
  debiting the cost account, not reversed). Refused with nothing to
  change, or with both `serial` and `clear_serial`. `asset` is the id or current tag. Journaled as `asset.tagged`,
  `asset.serial_set`.
- `books.asset_labels { assets?, skip? }` → `{ file_name, content_base64,
  tags }`: an Avery 5160/8160 label sheet (PDF): company, tag, QR code of
  the tag, description, serial. Default: every active tagged asset; `skip`
  reuses a partly used sheet.
- `books.asset_schedule { asset_id | tag, through? }`
  → `{ asset_id, description, cost, salvage_value, useful_life_months,
  status, schedule: [{ period, amount, accumulated, book_value }] }`;
  a disposed asset's ends on its disposal day. A schedule an earlier
  version's asset can't compute is a tool error saying why, never a
  panic.
- `books.list_assets { as_of? }` → `{ as_of, assets }` (default today, the
  company's time zone): per asset `in_service_date`, `acquired_date`,
  `cost`, `status`, `held` (acquired by `as_of` and not disposed),
  `accumulated_posted` (what the books carry: its depreciation runs, less
  reversals, a reversal of a reversal reinstating),
  `accumulated_scheduled_through` (what the schedule says is due by
  `as_of`), `unposted_depreciation` (the difference for this asset, its
  schedule less its runs: run `books.run_depreciation`; the register's
  total `unposted_depreciation` compares the schedules with the general
  ledger instead, so a hand entry in accumulated depreciation shows there,
  as `unattributed_accumulated`, not here), `book_value` (cost less
  posted: what the books carry), and `basis` (as
  `books.set_asset_tax_basis` reports it). `cost` is always the
  capitalized cost; `held` says whether the books carry it. Disposed by
  `as_of`: every other figure 0.00; acquired after `as_of`: those figures
  null. The rows are the register's (`fixed_asset_register`). An
  asset whose figures can't be computed (one an earlier version took)
  lists with them null and `schedule_error` saying why: one bad asset
  never fails the list.
  Before P3, `accumulated` was the schedule's figure and `book_value`
  cost less it.
- `books.set_asset_tax_basis { asset, rule, contributor_adjusted_basis?,
  fmv_at_contribution?, sec704c_method?, basis_source? }` → `{ asset_id, basis:
  { rule, contributor_adjusted_basis, fmv_at_contribution, fmv_recorded,
  tax_basis_723, depreciable_basis, built_in_gain_704c, sec704c_1c_note?,
  sec704c_method, source?, sources }, notes? }`. `rule`: `purchase` (the
  company bought it: depreciable basis is its cost, IRC §1012),
  `carryover_723` (a member contributed business property: the company's
  §723 basis is the member's adjusted basis, depreciated from), or
  `personal_use_conversion` (a member contributed personal-use property:
  the §723 basis as for carryover, depreciable basis the lesser of fair
  market value and that basis, Treas. Reg. §1.168(i)-4(b); D-41). A
  contribution needs `contributor_adjusted_basis` (cost plus improvements,
  less depreciation allowed or allowable, IRC §1016(a)) and
  `fmv_at_contribution` (a fact the human gives: never assumed from the
  cost); `built_in_gain_704c` = value − §723 basis at contribution (Treas.
  Reg. §1.704-3(a)(3)(ii)), with the IRC §704(c)(1)(C) note when it is a
  loss. `sec704c_method`: `traditional`, `traditional_with_curative`,
  `remedial` (Treas. Reg. §1.704-3(b)-(d)). `basis_source` `{ decided_by,
  decided_on?, citation?, note? }`: who gave the facts. `sources` names
  each figure's authority. The figures are at contribution or purchase;
  tax depreciation and what remains of a built-in gain later are the
  CPA's. The old names (`cost`, `carryover`, `contributor_basis`) are
  aliases for one release, noted. Journaled as `asset.basis_set` (the
  journal feature `asset_basis`: books an older version wrote refuse it,
  `format_upgrade_pending`, until the owner's format upgrade); a record
  written before P3 (`asset.tax_basis_set`) reads its value as the book
  cost (`fmv_recorded: false`).
- `books.correct_asset { asset, cost?, salvage_value?, useful_life_months?,
  in_service_date?, reason }` → `{ asset_id, tag, cost, salvage_value,
  useful_life_months, in_service_date, acquired_date, basis, notes? }`.
  Refused while the books carry depreciation for it (a run posted and not
  reversed), for a disposed asset, for a blank `reason`, and for an
  in-service date before its acquisition entry's date; `in_service` is the
  alias of `in_service_date` for one release.
  A recorded value at contribution, and so the built-in gain, never
  moves; a basis recorded before P3 has none and reads it as the book
  cost, so a cost correction moves its built-in gain, and `notes` says so.
- `books.run_depreciation { period: "YYYY-MM", asset_id? }`
  → `runs: [{ asset_id, description, period, amount, caught_up, entry_id,
  already_posted }]` and `skipped: [{ asset_id, description, reason }]`.
  A run posts what the schedule says is due through the month's end, so a
  month skipped or reversed is caught up in the month run (`caught_up`:
  the part beyond the month's own; a material miss is a restatement for
  the CPA, ASC 250), and never more than the schedule at a later month
  end. What the books carry is read entry by entry, each in its own
  month, so a month whose run was reversed after it ended posts nothing
  (it carried the month then; the run of the reversal's month catches it
  up). Idempotent per asset and period. Nothing due (the
  month rounds to nothing, a later run caught it up, the life is over and
  posted) posts nothing and is listed under `skipped` with why, as are a
  run-all's assets not yet in service and any whose schedule can't be
  computed. A run before the in-service date is refused.
- `books.dispose_asset { asset_id | tag, date, proceeds, proceeds_account,
  gain_loss_account, buyer?, confirmed?, receipt_attachment_ids? }` (the
  date never before the acquired date) →
  without confirmation: `{ status: "preview", asset_id, entry_id (0 in a
  preview), accumulated, book_value,
  proceeds, gain_loss, final_depreciation, excess_depreciation,
  catch_up_depreciation, lines, receipt_required }` and posts nothing;
  with `confirmed: true`: auto-posts the final depreciation (through the
  disposal day, held), then posts the balanced disposal entry. Book value
  is cost less the schedule through the disposal day, whatever the runs
  posted: depreciation a run posted past it (the disposal month's run is
  dated its end) is credited back to depreciation expense in the disposal
  entry (`excess_depreciation`); depreciation still due when the disposal
  month already has a run is debited there (`catch_up_depreciation`); the
  entry clears everything the runs carry. Refused, at the preview already,
  unless every run carries at the end of the disposal's month what it
  carries now: a run of a later month that stands (reverse it first), or
  a reversal (or re-instatement) of an earlier run dated after the
  disposal's month that a later one hasn't undone (undo it, on any open
  day). Subject to receipt policy (bill
  of sale); monthly depreciation is exempt as an allocation. `buyer` (a
  party) is the disposal entry's party; under `[books] parties =
  "require"` a sale with proceeds without one is refused before anything
  posts (`party_required`); a retirement with no proceeds and no buyer is
  marked as having no counterparty.
- `books.export { report: "fixed_asset_register", format: json | csv,
  as_of? }`: every asset acquired by the date (tag, serial, acquired and
  in-service dates, cost, accumulated depreciation scheduled and posted,
  book value as cost less posted, disposal, the basis facts and figures:
  rule, contributor's adjusted basis, value at contribution and whether
  it was recorded, §723 basis, depreciable basis, §704(c) built-in gain,
  method, who gave the facts), totals (`total_accumulated_scheduled`,
  `total_accumulated_posted`, `total_book_value`, `total_tax_basis_723`,
  `total_depreciable_basis`, `missing_basis`), and the general ledger's
  balances of the assets' cost and accumulated depreciation accounts, with
  `unposted_depreciation` (scheduled less books), `unattributed_accumulated`
  (books less the runs: an entry made by hand) and `unregistered_cost`
  (zero when register and books agree; `ledger_core::fixed_asset_register`).
  CSV columns: `tag, description, serial, acquired, in_service, cost,
  salvage, useful_life_months, accumulated_scheduled, accumulated_posted,
  book_value, status, proceeds, basis_rule, contributor_adjusted_basis,
  fmv_at_contribution, fmv_recorded, tax_basis_723, depreciable_basis,
  built_in_gain_704c, sec704c_1c, sec704c_method, basis_source`, then a
  TOTAL row; `sec704c_1c` is `true` for a built-in loss (IRC
  §704(c)(1)(C)), blank without a contribution basis.
- `books.show { asset }` opens an asset in the viewer; `page: "assets"`
  and `report: "fixed_asset_register"` open the register.

## Tools — schedules and recurring entries

Amortization spreads a prepaid expense (asset → expense) or deferred
revenue (liability → revenue) over the time it covers, posting on each
month end, to the minor unit (rules.md B-16). A new schedule's convention
is `books.schedule_convention` (unset: `daily`, D-11/D-37): `daily`
spreads by the day from `start`, the first day covered, as depreciation
does, so a mid-month start posts a small first and last month (a 12-month
policy from March 15 is thirteen month ends: 54.84, eleven of 100.00,
45.16 for 1,200.00); `monthly_even` is equal months, the last absorbing
rounding, and starts on the 1st (D-37: revenue is never earned early,
ASC 606-10-25-23). Schedules registered before P3 stay `monthly_even`
(mid-month starts included), forever. Like
depreciation, the schedule is the evidence: its postings are exempt from
the receipt policy (the original payment carries the receipt). Recurring
entries never post: they generate drafts, so every real receipt still goes
through the policy.

- `books.create_schedule { kind: "prepaid_expense" | "deferred_revenue",
  description, total, start, months, balance_account, pl_account,
  convention?: "daily" | "monthly_even" }` → `{ schedule_id, ...,
  convention, start, end, next, total, remaining, months: [{ date, amount,
  posted }], plan_error }`. Prepaid needs an asset and an expense account;
  deferred revenue a liability and a revenue account. `start` is the
  first day covered (for a monthly_even schedule an earlier version took
  mid-month, the 1st of its month), `end` the last day covered, `next`
  the next month end to post (null once every month has). Refused: a
  convention but those two (naming them); a `monthly_even` schedule not
  starting on the 1st (naming the 1st, and `daily`); one whose months
  aren't whole minor units (yen); `months` outside 1 to 600; one whose
  last day is after 9999-12-31; and, in books an older version wrote, a
  daily one (it needs the journal feature `schedule_convention`:
  `format_upgrade_pending` until the owner's format upgrade). `plan_error` is null, except for a schedule an earlier
  version accepted whose months run past the last date the books can
  compute: then it says why, `remaining` and `next` are null and `months`
  empty, and `books.run_schedules` refuses until it is dealt with.
- `books.list_schedules {}` → `{ schedules: [...] }`, same shape.
- `books.run_schedules { through?, confirmed? }` → `{ status: "preview" |
  "posted", postings: [{ schedule_id, date, description, amount,
  entry_id? }], skipped: [{ schedule_id, date, reason }] }`: every
  unposted month ending on or before `through` (default today).
  Idempotent; a month reversed within its month is due again; a month
  dated in a closed period, or whose posting was reversed after the month
  ended (posting it again would recognize it twice until then), is left
  and listed under `skipped` with why (book it in an open period by hand,
  or not at all), never a failed run.
- `books.create_recurring { description, frequency: "weekly" | "monthly" |
  "quarterly" | "yearly", start, end?, lines: [{ account, side, amount }] }`
  → `{ recurring_id, ..., generated: [] }`. Lines must balance. Monthly and
  longer occurrences are computed from the start date each time, so a
  month-end start stays on month ends (Jan 31, Feb 28, Mar 31).
- `books.list_recurring {}` → `{ recurring: [...] }`.
- `books.run_recurring { through? }` → `{ drafts: [{ recurring_id, date,
  entry_id }], skipped_closed_period: [{ recurring_id, date }] }`: one
  draft per occurrence on or before `through`, once; an occurrence dated
  in a closed period is not generated and is listed in
  `skipped_closed_period` for the human to decide.

## Tools — the viewer

- `books.show { entry_id | account | report | attachment_id | asset |
  party | project | reconciliation | search | page, start?, end?, columns?,
  compare? }`
  → `{ shown, viewer_open, note }`. Asks the human's viewer (`ledger-gui`)
  to open an
  entry, an account register or a report (for start/end dates if given),
  a stored file, an asset, a party (by id or name, resolved as in "Tools
  — parties"), a project (by id or name: viewer target `{show:
  "project", class_id}`; page `projects` its list), a reconciliation (by id: its account's page, with its
  statements and reconciliations; viewer target `{show:
  "reconciliation", id}`), a transactions search, or a page (`parties`
  among them). Exactly one target; entries, accounts, attachments,
  assets, parties, and reconciliations are checked to exist. Never
  journaled: the request is a line `{seq, at, target}` of the books' own
  `ops/show.jsonl` (at most 32 lines, replaced whole, so a reader sees
  each line whole), which the viewer reads and never writes. The server
  makes `ops/` and `ops/viewer.lock` when it opens the books (and on a
  request); the viewer holds a shared lock on `ops/viewer.lock`, through
  a read-only descriptor, while it has the books open, and
  `viewer_open` says whether one does. The note says "The viewer shows
  …", or "No viewer has these books open: ask the human to open it
  (opened within a couple of minutes, it shows …)".
  - **Dates (rules.md B-30).** For an account or a report, the dates are
    resolved by the one default (omitted: the fiscal year to date; only
    `end`: its fiscal year to it) and sent to the viewer resolved, so
    `shown` and the screen can't differ: "the register of 1000 Business
    Checking, 2026-01-01 to 2026-10-20", "the profit and loss report,
    2026-01-01 to 2026-10-20", "the trial balance report, as of
    2026-10-20" (trial balance, aging, fixed asset register, a balance
    sheet in one total column), "the vendor 1099 report, for 2026". A
    start after the end, given or derived, is refused and nothing is
    sent.
  - **Options a target ignores are refused**, naming what takes them:
    `start`, `end`, `columns` and `compare` for any target but an account
    or a report; `columns` and `compare` for an account; `columns` but for
    `profit_and_loss`, `balance_sheet` and `cash_flow`; `compare` but for
    those and `tax_summary`, and never with a split; `start` for a report
    as of a day (the trial balance, the agings, the fixed asset register,
    a balance sheet in one total column unless compared with the prior
    period) or for `vendor_1099` (its year is `end`'s); and a split into
    more columns than the viewer shows (`ops::MAX_REPORT_COLUMNS`, 240:
    past it the viewer shows the total).
  - The viewer takes a target's dates as sent; one it can't honour (text
    that isn't a date, a start after the end from an earlier writer or a
    hand-edited `ops/show.jsonl`, an unknown split) is ignored whole, as
    an unknown page is.

## Tools — integrity

- `books.verify { blobs? }` → `{ status: "ok" | "healed", events, head,
  healed?, blobs_verified?, database, other_databases?, checked }`
  (`database`: the projection file verified, one for each schema,
  `wanderbooks.v{N}.db`, or the older `wanderbooks.db` when it holds this
  version's schema; `other_databases`: other schemas' files beside it,
  left for the versions that use them). Verifies the journal's hash chain,
  replays it against the live ledger, and compares every table of the
  SQLite database row for row against a fresh projection of the journal.
  A drifted database is rebuilt from the journal (the old file kept as
  `wanderbooks.v{N}.mismatch.<time>`). With `blobs` (default true), re-hashes
  every stored file. The server runs the same check, and heals the same
  way, every time it starts.

## Tools — access

- `books.access_log { limit?, before?, token?, action?, outcome? }`
  (books:admin; read-only) → `{ records, total, more, head: { seq, hash }
  }`: who reached these books through the server (plan I-9), newest
  first, each record its token, client, tool or resource, a digest of
  its arguments and whether it ran (`ok`), failed (`error`) or was
  refused. `limit` is 1 to 500 (default 50); `before` pages back (records
  before that seq); `token` narrows to a token's name, `action` to `tool`,
  `resource` or `connect` (refused at the door), `outcome` to `ok`,
  `error` or `refused` (anything else is refused). `total` counts the
  whole log; `more` says older records match too. The log is read whole
  and its chain checked first: a damaged log is reported, never shown in
  part. Books served over stdio keep no access log: the tool is not
  offered there (and is refused if called).

## Tools — backups

Books served from the data root back themselves up (plan 3.7, D-30,
D-31): each keeps `archive.git`, a bare repository whose commits are its
journal, stored files, vault, keys and access log as they were (one
commit per backup, made under the store's lock, its message
`seq N head H` and, below, `book <id>`), pushed fast-forward only to the targets the settings
name: `backup.git_remote` (the owner's private repository) and
`backup.folder` (a repository `<book>.git` in a folder on another disk).
Not encrypted: the destinations keep it private. Backups run a little
after the books are last written (`--backup-quiet`, 30 s), when they
open, before they close, and on request; a failed one is retried after
15 minutes. Each target is drilled weekly: cloned, its chain replayed,
every stored file re-hashed, every TIN record the books name there with
its key, its head the one its commit records, its journal a byte prefix
of the live one (a failed drill is tried again at the server's next
hourly look, once 15 minutes have passed). A backup's tree is checked
before any of it is written out: plain files under the backed-up paths
only, never a link or a submodule. git runs with no hooks (in a
repository on this computer it pushes to as well), no ignore or
attributes rules of anyone's (a `.gitattributes` in a book's folder
refuses the backup) and no command-running transports; a repository on
this computer is used only if plain (a bare repository at that very
path, no settings of its own but where a copy came from, nothing sending
git elsewhere, no link where it or its parts are), and a backup folder that took backups and is gone is
reported as a disk not connected, never made again (restored books count
the backups of the books they were). Books restored beside the books they
were (under another id) don't push to those books' repository, which
would stop theirs: `books.backup` says it is paused for them until
another is named. A damaged backup record is set aside and begun again;
it never keeps the books shut. One book's drill holds the others'
backups until it ends (a backup at close waits at most a minute for
it). A target
whose push failed is left out of the backups after writes for 15
minutes, then tried again, written to or not; the other targets don't
wait for it. Every
commit, push and drill is a record in the book's `ops/backup.jsonl` (a
chain log; a backup never moves the books' head), each target's records
keyed by the target itself. Setting `backup.*` (books.set_setting, or
books.import_settings with a `[backup]` table) needs books:admin, as
backing up does: a backup is the whole book, its vault and key included.
Over stdio neither tool is offered.

- `books.backup {}` (books:read) → `{ head_seq, targets: [{ target:
  "repository" | "folder", at, last_ok_at, through_seq, stale,
  last_error, last_drill_at, drill_error, paused }], note }`. `paused`:
  not backed up there, these books being restored as a copy of others
  (its `last_error` says so). `stale`: the target
  lacks the head, and the first try to back up what it lacks (since its
  last good backup, or ever) is more than a day old. `last_error`: its
  last push's, or a later failed commit's. `note`
  says what to set when no target is. `books.start` adds a next step when
  none is set up, or one is stale, failing, paused or failed its drill.
- `books.back_up_now { new_disk? }` (books:admin) → the same, after
  backing up to every target now. `new_disk`: the backup folder's disk is
  new, so a folder that took these books' backups and holds none now is
  begun again (otherwise it is a disk not connected). The one tool whose
  `openWorldHint` is true. A target paused for books restored as a copy
  of others shows `paused` (and is no failure to retry).

Restoring is the owner's, from a terminal: `wanderbooks books restore
BACKUP [--id ID] [--replace] [--yes]` (the admin socket's `restore`,
previewed then confirmed against the previewed head; without `--id`, the
books keep the id of the books the backup holds, which the preview names,
else the repository's name; under another id they are a copy, and don't
push to those books' repository). The backup is
cloned into the root, proven whole as a drill proves it (but for the
live-journal check: there may be no live books), and built as an
adoption builds; it arrives as new books, or replaces books (`--replace`,
which are let go and moved whole into `legacy/<id>-<time>/`). The
restored books keep the backup's history, so their next backups carry
on from it.

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

What counts toward a vendor's 1099 is what left the books' cash accounts
(`books.cash_accounts`) and financing accounts (`books.financing_accounts`)
on entries tagged with the vendor, classified by how each payment was made
(rules.md B-19; plan P3, defects 12, 18 and 23; D-40):

- out of a cash account with no payment method recorded: counts
  (presumed check, ACH or transfer; tag a debit-card payment with the
  card);
- by card or payment network (a method of that kind, or a company card's
  account): excluded, the processor's Form 1099-K, whoever's card it was
  (Treas. Reg. §1.6041-1(a)(1)(iv));
- through a financing account (a member paid), or the account of a
  member's or a platform's method, with no method recorded:
  undetermined, until it is (their card: excluded; their own cash or
  check: counts, the company being the payor, §1.6041-1(e)(1)).

Money moved between the company's own sources (the card bill, a member
reimbursed, a loan paid down) pays no one; money received from a vendor
for anything but a refund of what was paid (a sale, its payment of an
invoice, a disposal, a loan) isn't netted and is listed.
Books with no cash accounts named are refused. Thresholds: $600 for
payments through 2025, $2,000 from 2026 (Pub. L. 119-21 §70433; indexed
for inflation after 2026); always overridable, always for the CPA to
confirm. The books keep whether a W-9 is on file and the TIN's last four;
full TINs live only in the TIN vault ("Tools — the TIN vault").

Each vendor has a 1099 decision: `nec`, `misc`, `not_reportable` with a
reason kind (`corporation`, `tax_exempt`, `united_states`, `state`,
`foreign_government`, `foreign_person`, `merchandise`, `employee`, `other`)
and the facts in words, or `undetermined` (also what a vendor nobody
decided on reads as). Each is recorded with who decided and when, and the
date it takes effect, or none (from the beginning: the vendor's status
all along, and every decision written before decisions were kept). Each
payment takes the decision in force on its date (its reversal chain's
first entry's).

- `books.list_vendors {}` → `{ vendors: [{ id, name, form_1099?,
  decision_1099, decisions_1099, tin_on_file, party_id }] }`.
  `decision_1099` is the decision in force today, `{ value, reason_kind?,
  reason?, authority?, effective?, decided_by?, decided_on?, citation?,
  note? }` (`authority`: the rule that exempts the reason kind; no
  `decided_by`: nobody decided); `decisions_1099` every recorded
  decision, in date order; `form_1099` "1099-NEC" or "1099-MISC" when
  today's decision files one.
- `books.set_vendor_1099 { vendor, form_1099?, reason_kind?, reason?,
  effective?, tin_on_file?, decided_by?, decided_on?, citation?, note? }`
  → the vendor, with `changed` ("set", "removed", "unchanged") and
  `warnings`. `vendor` is an id or a name. With `form_1099` (`nec`,
  `misc`, `not_reportable`, `undetermined`), `decided_by` is required
  (`decided_on` defaults to today); `not_reportable` takes `reason_kind`
  and `reason` (`other` a `citation` too), no other value takes them.
  "none" is refused, saying what to say instead; paid by card is never a
  reason. `effective` (YYYY-MM-DD) only when the payee's status changed:
  an LLC's corporate election from its Form 8832 date; a payee that
  incorporates is a new vendor. A dated decision with nothing decided
  before it is refused; one the same as the decision before its date
  takes out any decision recorded for that date ("removed"); recorded
  again exactly so (who decided and when included), nothing is journaled
  but a changed W-9 flag ("unchanged"). Without `form_1099`, `tin_on_file`
  alone marks the W-9, keeping the decision and who made it (`changed` is
  then "unchanged", with no warnings); `reason_kind`, `reason` or
  `effective` without `form_1099` are refused, and so is a call with
  neither `form_1099` nor `tin_on_file`; `tin_on_file` omitted keeps the
  flag. `warnings` names each
  year (and its entries) whose payments the change reaches in a closed
  period or after that year's 1099s were due (January 31 of the next
  year): tell the human, a filed 1099 may need correcting. Journaled as
  `vendor.1099_set` (a W-9 flag alone as `vendor.w9_flag_set`), under the
  journal feature `vendor_1099_decisions` (books an older version wrote
  refuse it, `format_upgrade_pending`, until their format upgrade: B-27).
- `books.add_instrument { kind, label, last4?, owner_party?, account }` →
  `{ id, kind, card_or_network, label, last4?, owner_party?, account_id,
  account, warnings }`. `kind`: `bank` (the company's checks, ACH, wires,
  bill-pay), `business_card` (a company card the payee charged),
  `member_card` (a member's personal card the payee charged),
  `member_cash_or_check` (a member's own cash, check, ACH, Zelle, or a
  friends-and-family transfer), `platform` (paid to the payee as a seller
  through PayPal or Venmo goods and services; never ACH, bill-pay, Zelle or
  friends-and-family). `account` (id or code) is what it pays through: the
  bank account, the card's liability, Due to Members for a member's own
  card or cash; an asset or liability, not a header, not payables or
  receivables. `label` is unique (ignoring case); `last4` exactly four
  digits. Journaled as `instrument.added` (feature `payment_instruments`).
- `books.instruments {}` → `{ instruments: [...] }`, by id. Read-only.
- `books.set_entry_instrument { entry_id, instrument? }` → the entry (as
  `books.get_entry`), with `changed` and `warnings` (as above). An id or
  label; omit to clear. Metadata only: drafts and posted entries alike;
  the entry and its reversals are tagged together. Journaled as
  `entry.instrument_set`.
- `books.set_entry_vendor { entry_id, vendor? }` → the entry (as
  `books.get_entry`, which shows `vendor`, and `instrument` and
  `instrument_id`). Omitting `vendor` clears the tag. On an entry naming
  no party or one, the vendor's party becomes its party (clearing a
  no-party mark), and clearing the tag clears the party too when it was
  the vendor's. Metadata only:
  posted entries may be tagged. `books.draft_entry` takes `vendor` and
  `instrument` too; bills and their payments are tagged automatically.
  `books.set_entry_party` tags the vendor too when the party has a vendor
  record, so tagging by party counts for 1099s the same way. On an entry
  naming several parties the parties are `books.set_entry_party`'s:
  tagging the vendor of a party already named keeps every share as it
  is; tagging a vendor whose party isn't named, or clearing the tag while
  its party is named, is refused (drop or add the party with `parties`
  instead). An entry and its reversal always carry the same tags.
- `books.vendor_1099 { year, threshold?, include_tins?, csv?,
  cash_accounts?, financing_accounts?, through? }` →
  `{ year, threshold, vendors: [{ vendor_id, name, form_1099?,
  decision_1099, decisions_in_year, tin_on_file, paid, excluded_card,
  undetermined, not_reportable, status, decision_needed, reportable,
  party_id?, tin_last4?, tin?, tin_type?, tin_note? }], missing_w9,
  excluded_card_payments, undetermined_payments, other_receipts,
  untagged_entries, unresolved, notes, warning?, csv? }`. `paid` is what
  counts toward the 1099; `excluded_card` what a card or network paid;
  `undetermined` what waits on its payment method; `not_reportable` what
  was paid while the vendor was decided not reportable. `status`:
  `reportable` (file one: what counts reaches the threshold however the
  unknowns settle), `under_threshold`, `not_reportable`, or
  `undetermined` (a decision or a method is needed first;
  `decision_needed` says which); `reportable` is `status = reportable`.
  Each payment list is `[{ entry_id, date, vendor_id, vendor, amount,
  instrument? }]`. `missing_w9` names the vendors a 1099 is or may be
  ("(may be reportable)") due for without a W-9. The notes are quiet:
  untagged entries are one count (those that paid money out for
  something) with the tool to tag them. For a year whose threshold
  wanderbooks doesn't know (after 2026 until the indexed amount is added)
  it refuses with what to do; pass `threshold` explicitly once the
  figure is confirmed. Payments are counted out of `cash_accounts` and
  `financing_accounts` (account codes; each defaults to the `[books]`
  setting, the CPA package's rule: `ledger_package::Settings::with_accounts`;
  a code the chart lacks is refused), from January 1 of `year` through
  `through` when given (`YYYY-MM-DD`, a day of `year`: the worksheet as of
  a day, rules.md B-33), else the whole year. On an entry naming several
  parties, the buckets count only the vendor's resolved share of the
  payment, split across them to the cent (rules.md B-19); an entry whose
  shares don't resolve counts in full and `notes` names it. `tin_last4`
  is the party's TIN facts' last four, else its W-9 facts'
  (`Party::tin_last4`). With `include_tins: true` (default false) each
  line carries `tin`, the full TIN formatted as the IRS writes it (123-45-6789;
  12-3456789 for an EIN), and `tin_type`, read from the vault, or
  `tin_note` saying why not (not in the vault, the vault locked, a record
  whose last four or hash disagree with the books); `warning` then says
  "This worksheet holds full TINs, for the CPA; keep it out of the
  books.". This is the only tool output that ever holds a full TIN.
  `csv: true` (default false) also hands the worksheet over as a CSV file,
  `csv` (a `HandedFile`, "Tools — the TIN vault"), named
  `1099-worksheet-<year>.csv`, or `1099-worksheet-<year>-with-tins.csv`
  with `include_tins` (TINs only then): `vendor_id, name, decision_1099,
  form_1099, reason_kind, reason, decided_by, decided_on, effective,
  decisions_in_year, paid, excluded_card, undetermined, not_reportable,
  status, reportable, w9_on_file, tin_last4` (and `tin, tin_type,
  tin_note`). The CPA package carries the same CSV: one list of cash
  accounts and one of financing accounts per package, the request's, else
  the settings', for every file in it (R18), the 1099 worksheet through
  the package's `as_of`; so `books.vendor_1099` given the package's
  `cash_accounts`, `financing_accounts` and `through` = `as_of` (and its
  `include_tins`) hands over the package's `1099-worksheet-<year>.csv`
  byte for byte (the tool names one with TINs `-with-tins`; checked by `ledger-mcp` `oracle_mcp.rs`
  `cpa_package_holds_the_year_and_its_evidence`).

## Tools — parties

The directory of everyone the company deals with (`ledger_core::parties`;
design in `docs/parties-and-attachments.md`): vendors, contractors,
landlords, customers, payout platforms, tax agencies, banks and card
issuers, members, employees. Vendor and customer *records* (bills,
invoices, aging, 1099 marks) stay where they are; each links to exactly one
party. `books.add_vendor` and `books.add_customer` link their record to the
party of the same name (ignoring case and surrounding space, not archived)
that has no such record yet, or make a new party.

**References.** A `party` argument is an id or a name: the name ignoring
case and surrounding space, among parties not archived (an archived party
goes by id). A number that is one party's id and another party's name is
refused, never guessed.

**Roles:** `vendor`, `contractor`, `customer`, `employee`, `member`,
`payout_platform`, `tax_agency`, `government_agency` (regulators and
registries that don't tax: a division of corporations, a licensing board),
`financial_institution`, `landlord`.
Vendor and customer follow the records and are never set by hand.

**Rules the engine enforces** (each refusal changes nothing, the journal
byte for byte): no party field holds a full TIN (a run of nine digits,
single spaces or hyphens allowed between them); a W-9's TIN is its last
four digits only; W-9 facts need a 1099 role (contractor or landlord), W-4
facts the employee role; names are unique among parties not archived;
texts are at most 500 characters, no control characters.

- `books.add_party { name, roles? }` → the party (below). Journaled as
  `party.added`.
- `books.update_party { party, name?, display_name?, website?, address?,
  notes?, archived?, creditor_of? }` → the party. Give at least one field;
  `""` clears `display_name`, `website`, `address`, or `notes`;
  `creditor_of` (account codes or ids) replaces the party's list of the
  liabilities the company owes it itself (below). Every field is checked
  (in order, on a rehearsal) before any is written; each is journaled as
  its own `party.updated`, in the order name, display_name, website,
  address, notes, archived, then one `party.creditor_set` per account
  added or dropped. Un-archiving is refused while another party has the
  name.
- `books.set_party_roles { party, add?, remove? }` → the party. Adds apply
  before removes (so a contractor's W-9 survives a swap to landlord); roles
  it already has, or lacks, are skipped; each change is journaled as
  `party.role_set`. Removing the last 1099 role with W-9 facts on file, or
  employee with W-4 facts, is refused: clear the facts first.
- `books.set_w9 { party, received_on, entity_type, exempt_payee_code?,
  fatca_code?, tin_last4, kept_at, tin?, tin_type?, w9_upload_id?,
  w9_content_base64?, w9_sha256?, w9_file_name? }` or `{ party, clear:
  true }` → the party. Journaled as `party.w9_set`. A party with a
  vendor record also gets the record's `tin_on_file` set to match
  (journaled as `vendor.w9_flag_set`, its 1099 decision and who made it
  kept), so `books.vendor_1099`'s `missing_w9` follows.
  `tin` and `tin_type` put the full TIN in the TIN vault, and the signed
  form (`w9_upload_id`, or `w9_content_base64` with `w9_sha256`, as in
  "Tools — receipts & documents") goes there too (below); with a TIN,
  `tin_last4` comes from it (a given one must agree); with a TIN or the
  form, `kept_at` defaults to `"wanderbooks vault"`.
- `books.set_w4 { party, received_on, state, kept_at }` or `{ party, clear:
  true }` → the party. Journaled as `party.w4_set`.
- `books.set_entry_party { entry_id, party?, parties?, no_party_reason? }`
  → the entry (as `books.get_entry`). `party` null or omitted clears the
  tag. A party with a vendor record also tags the entry's vendor; one
  without, or clearing, clears the vendor tag. Metadata only; posted
  entries may be tagged. Tagging an entry that was reversed, or a
  reversal, tags the other too (the whole reversal chain), so each
  party's part of the pair still nets to zero. Journaled as
  `entry.party_set`.
  `no_party_reason` (with no party) marks an entry as having no
  counterparty (`entry.no_party`).
- **Several parties:** `parties: [{ party, amount? | percent? }]`, in
  place of `party`, names every counterparty with its share; `[]` clears.
  Journaled as `entry.parties_set { entry_id, parties: [{ party_id, share
  }] }`, `share` being `{"amount": "30.00"}`, `{"percent": "60"}`, or
  `"derived"`. Once the entry is posted (at `post_entry`, or tagging a
  posted entry), a derived split among several parties is journaled again
  as the exact amounts it resolves to (a second `entry.parties_set`), so a
  later change to `[[company.members]]` never re-splits posted history; it
  stays derived when the shares don't resolve or a part is zero. Each share, of what the entry moved (its `entry_flow` net;
  for an entry moving no money, like property contributed, its total):
  - `amount`: exactly that, positive, in cents; the amounts must sum to
    what it moved.
  - `percent`: that percent; the percents must sum to exactly 100. Each
    part is rounded down to the cent and the leftover cents go to the
    first party listed.
  - neither (derived): a sole party gets all of it. Members named
    together (parties with the member role named in `[[company.members]]`)
    get what their own capital and distributions lines on the entry say,
    when it has any (these must account for all it moved); else their
    `[[company.members]]` shares, rescaled to the members named, rounded as
    percents are. A derived share for a non-member among several is
    refused.
  Every party gives the same kind of share. Refused, journaling nothing:
  `party` with `parties`, or either with `no_party_reason`; a share giving
  both `amount` and `percent`; a party named twice, an unknown party, mixed kinds, percents not summing
  to 100, an amount finer than a cent, two parties with vendor records (a
  1099 counts a payment toward one vendor; the one vendor party's record
  is tagged and counts the whole payment), and, once the entry has
  balanced lines, shares that don't resolve. A draft's shares are checked
  against its lines again at `post_entry`. Under `[books] parties =
  "require"`, any party (or the no-party mark) satisfies the check.
  Reversals carry every share. There is no "joint" party: a joint member
  transaction names both members and lets the shares derive.
- `books.parties { role?, include_archived?, year?, basis? }` → `{ year,
  basis, parties: [{ party_id, name, label, roles, vendor_id?,
  customer_id?, w9_on_file, tin_last4?, w4_on_file, archived, paid,
  received }] }`, by id. `tin_last4` is the TIN's last four when the books
  know it: its TIN facts' (`books.set_tin`; a member's SSN for the K-1),
  else its W-9 facts' (the two never disagree), null with neither, kept
  after `books.purge_tin` (P3 defect 11: it read W-9 facts only, so a
  member's showed as missing).
- `books.party { party, year?, basis? }` → `{ party, year, totals,
  entries, invoices, bills, payroll, projects }`: the facts; the calendar year's
  totals (default this year); every entry it is on (drafts included,
  newest first); its invoices and bills (`{ id, date, due_date, total,
  paid, balance, status, entry_id }`); and the payroll lines naming it
  (`{ run_id, date, entry_id, gross_pay, net_pay }`); and the projects for
  or with it, by name (`{ id, name }`, rules.md B-37).
- `basis` (both): `"cash"` (the default) or `"all"`; any other word is
  refused (`basis is "cash" or "all"`). Echoed in the output. Not the
  books' accounting method (`books.accounting_basis`).

The party (every mutating party tool returns it): `{ party_id, name,
label, display_name?, roles, website?, address?, notes?, archived,
vendor_id?, customer_id?, w9?: { received_on, entity_type,
exempt_payee_code?, fatca_code?, tin_last4, kept_at, in_vault }, w4?: { received_on,
state, kept_at }, tin?, creditor_of }`. `label` is the display name, else
the name; `roles` include vendor and customer when it has those records.
`creditor_of` lists, by code, the liability accounts the company owes the
party itself on: the card it issued, the loan it made, what the members
advanced (each member). `books.update_party { creditor_of: [codes] }`
replaces the list (`[]` clears it), journaled as `party.creditor_set {id,
account_id, on}`, one line per account added or dropped, under the
journal feature `party_creditors` (books an older version wrote refuse
the whole change, `format_upgrade_pending`, until their format upgrade:
B-27); refused, journaling nothing, for an account that isn't a
liability, is a placeholder, or is payables or a payroll withholding
(owed to many).

**Each party's part of an entry** (`books.get_entry`, the
`books.post_entry` preview, `books.set_entry_party`: `parties: [{ party_id,
party, share, amount?, percent?, resolved, direction, cash, in_kind }]`,
rules.md B-35):

- `resolved`: signed as a line on the party's own account would be
  (debit-positive): positive, the company paid it or gave it value;
  negative, the company received from it. A reversal's is exactly its
  original's negated, never read from its own lines.
- `direction`: `paid`, `received` or `neither` (a promise not yet settled:
  a bill, an invoice, a fee on the party's own card, a distribution still
  owed; or an entry that moved nothing with anyone, like a close).
- `cash`: what of it moved as money; `in_kind`: what moved as property.
  Both `"0.00"` for `neither`; `resolved − cash − in_kind` is a promise,
  counted on neither basis. All four are null exactly when the shares
  don't resolve (`parties_error` says why).
- What the shares split (B-35 item 3): a close, its parties' own-account
  movement, neither; a reversal, its original's negated; an entry naming
  only members that moves equity, what moved their capital (a
  contribution received: the property in it, asset accounts that are
  neither cash nor receivables, the K-1 worksheet's test, in kind and the
  rest cash; a distribution paid: the money it paid as cash, the property
  it gave in kind, the rest still owed, counted when it is paid); money
  moved, the money (paid or
  received); otherwise the parties' own-account movement (payables,
  receivables, a liability owed to them: a bill −, an invoice +), else the
  total debits, neither.

**Totals** (`books.party` `totals`; `paid` and `received` in
`books.parties`), from the journal:

- *The party's entries:* tagged with the party; tagged with its vendor
  record (bills and their payments); its customer record's invoices and
  the payments received on them; and reversals of any of these. Only
  posted entries dated in the year count.
- *Money accounts* (B-35 item 5, P3 defect 24): the `[books]
  cash_accounts` (every asset account but receivables when none are set),
  the cards (the business-card payment methods' liability accounts,
  `books.instruments`; while the books record none, the liability
  accounts whose names say "card" and not "gift"; never a member's
  card or a platform's account: the one rule the reconciliation reminders
  and the viewer read), and the `[books]
  financing_accounts` (what the members advanced, loans). Any other
  liability (an accrued expense, deferred revenue, gift cards sold,
  distributions payable) is a promise, counted when it is settled, never
  also when it is booked. A liability owed to a party the entry names (`creditor_of`)
  isn't money for that entry: a credit to it never counts as paying the
  party.
- `paid` / `received`: each entry's part for the party, added up on its
  root original's side (a reversal subtracts there, and reversing the
  reversal adds back): on basis `cash`, the `cash` parts; on basis `all`,
  `cash` plus `in_kind`. A cash payment, a card charge, a member paying
  for the company: paid to the payee then; cash paying one of the money
  liabilities down: paid to its creditor (a reimbursement to the members,
  a card payment to the issuer); cash raised by one: received from the
  lender. A bill or invoice counts on neither basis (a promise is not
  payment); a transfer between the company's own cash accounts nets to
  zero. An entry naming several parties counts this party's part only (an
  entry whose shares don't resolve counts for none). For a member, basis
  `all` over the entries that name only members and move their capital
  (and the payments of distributions declared to them) is the statement
  of members' equity's contributions and distributions, and basis `cash`
  their contributions is the K-1 worksheet's cash contributions (B-35
  item 4 gives the conditions: a distribution declared and not yet paid
  is in the statement only). The year is the whole
  calendar year; the viewer's party page and directory show these same
  numbers, labelled "cash basis", with the in-kind figures beside them.
- `invoiced` / `billed`: its invoices' and bills' totals dated in the year.
  `owed` / `receivable`: its open bills' and invoices' balances now (every
  year).
- `amount_1099` (vendor parties only): what `books.vendor_1099` counts
  toward its vendor record's 1099 in the year (its `paid`: card and
  network payments excluded, member payments with no method recorded
  undetermined; absent when the worksheet can't be made); `form_1099` and
  `decision_1099` are the record's decision in force today.
- `gross_pay` / `net_pay`: payroll lines naming it, in the year.

## Tools — the TIN vault

Full TINs (SSN, EIN, ITIN) and signed W-9 forms, kept so what the CPA
gets is complete (every member's TIN for their K-1, every payee's for
their 1099), never in the books (`ledger-vault`; design in
`docs/parties-and-attachments.md` "The TIN vault"; B-26). DECISION
(the owner, 2026-10-15): the vault holds everyone's TIN, members included,
and the CPA package carries them.

- **Where:** `vault/` in the books dir (the parent of the data dir, beside
  `wanderbooks.toml`; git-ignored), one file per record, each encrypted
  (XChaCha20-Poly1305) and bound to its party. **Key:** the file named by
  `[vault] key_file` in `wanderbooks.toml`, else
  `~/.config/wanderbooks/vault.key` (`$XDG_CONFIG_HOME`), mode 0600,
  created the first time a TIN or form is stored, never when reading. A
  key file inside the books dir is refused, and so is storing into books
  kept in git (a `.git` beside them) whose `.gitignore` doesn't list
  `vault/`. Without the key the vault is
  locked: the books open and verify; the worksheet says the TINs are
  missing.
- **Never out:** no tool output, error, or log holds more than a TIN's
  last four, except `books.vendor_1099 { include_tins: true }`; the CPA
  package carries them only in its zip, handed over as an export (or
  inline over stdio), never written to a path.
  The tool arguments that take a TIN never echo it, even when refused.
- **TIN facts:** any party (any role) may have `tin` facts:
  `{ tin_type: "ssn" | "ein" | "itin", last4, vault_record?, purged? }`,
  journaled as `party.tin_set {id, facts|null}` and shown in every party
  output as `tin: { tin_type, last4, in_vault, purged? }`. They never
  disagree with the party's W-9 facts: both on file means the same last
  four (the engine refuses otherwise).
- `books.set_tin { party, tin, tin_type }` → the party. Validates the TIN
  as `books.set_w9` does, checks the facts on a copy of the ledger, writes
  the TIN to the vault (the same key and git-ignore checks), and journals
  `party.tin_set` with the record's hash. Any role. A party whose W-9
  facts name another last four is refused (a payee's new TIN comes with
  its W-9: `books.set_w9`); one whose W-9 facts agree gets their
  `vault_record` updated too (`party.w9_set`).
- `books.set_w9 { ..., tin, tin_type, w9_upload_id? | w9_content_base64? +
  w9_sha256?, w9_file_name? }` (above): validates the TIN against its type (nine digits,
  hyphens allowed; an ITIN starts with 9 and has an IRS group in digits
  4-5; an SSN or EIN doesn't start with 9), checks the facts on a copy of
  the ledger, then writes the vault and journals the facts with
  `vault_record`, the SHA-256 of the TIN record (of the form's record when
  only a form is given). With a TIN it also journals the party's TIN facts
  (`party.tin_set`, as `books.set_tin`); a TIN with a new last four clears
  the old TIN facts first (`party.tin_set` null), so the two never
  disagree. The form (a PDF, PNG or JPEG, judged by its own first bytes,
  whatever it is called; an upload of purpose `w9`, at most 25 MiB, or
  inline at most 256 KiB) goes to the vault only, never the attachment
  store. Its file name is `w9_file_name` if given, else the upload's,
  else `w9.pdf`, `w9.png` or `w9.jpg` by its type, checked as any filed
  name; `w9_file_name` without a W-9 is refused. Facts given later without a TIN
  keep the vault record when their last four agree with the vault's TIN,
  and are refused when they don't (pass the new TIN, or purge). Clearing
  the facts (`clear: true`) is refused while the vault holds the party's
  TIN or form: purge first.
- `books.get_w9 { party }` → `{ party_id, file }`: the party's form from
  the vault, handed over. Journals nothing.
- **Handed files** (D-29): no tool takes a path. A file the books make
  for an agent (`books.get_w9`'s form, `books.vendor_1099`'s `csv`,
  `books.export_cpa_package`'s zip) comes back as a `HandedFile {
  file_name, mime_type, size_bytes, sha256, download?, expires_at?,
  content_base64? }`. On books served by `wanderbooks mcp` it is an
  export, kept 15 minutes for the token that asked (at most 16 per
  token, swept like uploads), and `download` is the command that saves
  it in a folder the human names: `wanderbooks download --book B
  [--token NAME] ex_… --to <a folder>` (`GET /b/{book}/x/{export}`,
  "Transports"). Over stdio it comes back inline as `content_base64`.
  A file with full TINs never goes into the books: the server keeps, beside the vault, the SHA-256 of every
  file it read or wrote with full TINs (the W-9 `books.set_w9` reads,
  recorded once the call has passed every check, just before the vault
  keeps it, so a refused call marks no file; each of these outputs; each
  file inside a CPA package that holds a TIN), shared by every session and
  kept across restarts, and every filing tool (`books.attach_receipt`,
  `books.file_document`) refuses those exact files and any text file (CSV,
  TSV, text, JSON, IIF, XML, HTML, Markdown, by type, name or UTF-8 or
  UTF-16 decoding, UTF-16 with a byte-order mark or NUL-interleaved
  without one) holding a full TIN, storing nothing. A text file holds one
  when it has a TIN the vault holds, anywhere, as nine digits, 3-2-4
  joined by hyphens, spaces or dots, or 2-7 joined by a hyphen or a space
  (compared on the server; a vault that can't be opened leaves the next
  test alone), or a stand-alone number printed as an SSN or EIN is
  (123-45-6789, 12-3456789; no digit, and no hyphen or dot followed by a
  digit, beside it). Bank exports pass: routing and account numbers, long
  references and dates next to amounts are not TINs (rules.md B-26). No
  probing: a text with more than two TIN-shaped numbers ending in the last
  four of a TIN the books know is refused before the vault is opened, in
  the same words whether or not one is that TIN, so the vault is compared
  against at most two candidates a filing (rules.md B-26). An altered copy
  of a binary file (a W-9 PDF re-saved or re-scanned), and a TIN the vault
  doesn't hold written another way, are not detected: P7 takes full
  TINs out of the agent's reach.
- `books.purge_tin { party, reason, confirmed? }` → `{ status: "preview" |
  "purged", party_id, name, tin_in_vault, w9_file_in_vault, tin_last4?,
  journaled, note }`. Without `confirmed`, a preview that changes nothing.
  Confirmed, deletes the party's vault records (needs no key) and, when the
  party has W-9 facts, journals `party.w9_set` with `vault_record` cleared,
  the last four kept, and `kept_at` saying when and why it was purged;
  when it has TIN facts, `party.tin_set` likewise, with `purged` saying
  when and why. Refused when the reason is blank or there is nothing to
  purge.

## Tools — attachments

A stored file (a receipt or a document) links to any number of subjects;
a subject is `{ kind, id? }`: `entry` (id), `account` (code or id),
`asset` (tag or id), `party` (id or name), `project` (id or name: its
contract, spec, pitch; journal feature `projects`), `invoice`, `bill`, `period`,
`payroll_run`, `schedule`, `reconciliation` (ids), or `company` (no id).
Every subject is checked to exist. In outputs a subject is `{ kind, id?,
label }` (`id` absent for the company; for an account, its id, with the
code in `label`). A file linked to an entry is one of the entry's receipts:
the receipt policy counts it.

- `books.link_attachment { attachment_id, subject }` → `{ attachment_id,
  links }`. Journaled as `attachment.linked`. Refused for an unknown or
  withdrawn attachment, a subject that doesn't exist, or one it is linked
  to already (or filed on).
- `books.unlink_attachment { attachment_id, subject, reason }` → `{
  attachment_id, links }`. Journaled as `attachment.unlinked` with the
  reason; the file stays filed. Refused when the reason is blank, it isn't
  linked there, it is filed on that entry (`books.unlink_receipt`), it is
  a posted entry's last receipt under the receipt policy
  (`receipt_required`), or it is the statement of a reconciliation not
  voided.
- `books.attachments { subject }` → `{ subject, attachments: [...] }`: the
  files on a subject (an entry's: filed on it and linked to it), not
  withdrawn, each as in `books.list_receipts`.
- `link_to: [subject]` on `books.attach_receipt` and `books.file_document`
  links the new file in the same call, each link its own
  `attachment.linked`. All or nothing: every subject is checked (exists,
  named once, not the entry it is filed on, its journal feature declared:
  a `project` needs `projects`) before anything is filed, so a refusal
  leaves no file filed without its links.

## Tools — receipts & documents

A file comes in one of two ways, named in the tool that files it
(`books.attach_receipt`, `books.file_document`, `books.set_w9`,
`books.import_bank_csv`, `books.import_chart_csv`): `upload_id`, an
upload made with `books.begin_upload` (any file up to 25 MiB; its bytes
never pass through the agent), or `content_base64` with its `sha256`
(at most 256 KiB decoded; longer text is refused before it is decoded).
Never both, never neither. Inline bytes that aren't base64, or whose
SHA-256 isn't the one given, are refused (guard `base64_corrupt`): send
the file as an upload. Uploads exist only for books served from the
data root (the server's socket carries the bytes); over stdio, only
inline, and the refusal says so and to tell the human. On the socket an
MCP request is at most 1 MiB. A file's name and type are checked when
it is filed (every filing tool, `books.begin_upload`, and the W-9's
name): a name of 1 to 255 bytes, with no `/` or `\`, not `.` or `..`,
and no control or invisible (Unicode format) characters; a MIME type
`type/subtype` (parameters allowed) of visible ASCII, at most 127 bytes.

- `books.begin_upload { sha256, size, mime_type, file_name, purpose:
  "receipt" | "document" | "import" | "w9" }` → `{ upload_id?, command?,
  expires_at?, max_bytes, existing_attachment_id? }`. Makes a grant for
  the calling token: one SHA-256, one size (at most 25 MiB; empty
  refused), one purpose, lasting 15 minutes; at most 16 under way per
  token. A tool takes only an upload of its own purpose:
  `books.attach_receipt` and `books.file_document` take `receipt` or
  `document`, the CSV imports `import`, `books.set_w9` `w9`; another
  tool refuses it and leaves it for the right one. `command` is
  `wanderbooks upload --book B <upload_id> -- <the file's path>` (with
  `--token NAME` when the caller's token isn't the default `agent`),
  which sends the bytes (`PUT /b/{book}/u/{upload}`, "Transports"); the
  agent runs it, then names `upload_id` in the tool that files it.
  Bytes that aren't the size and SHA-256 declared are refused and
  nothing is kept. An upload's bytes are sent once, only by the token
  that began it, and used once: the call that takes them holds them
  (hashed again) and spends them only when they are in the books (an
  import: once its rehearsal passed); a call refused after taking them,
  or one that panics, gives them back, and a server stopped mid-call
  gives them back when the books next open. Lapsed grants are refused,
  and swept when the books open, whenever a grant is made, and every
  10 seconds. For `receipt` or `document`, when an attachment not withdrawn
  already holds those bytes, no grant is made: `existing_attachment_id`
  names it (link it, `books.link_attachment`, instead of filing a copy);
  for `import` and `w9` a grant is always made. Refusals never name a
  path.
- `books.attach_receipt { entry_id?, file_name?, mime_type?, upload_id? |
  content_base64? + sha256?, description?, link_to? }` → `{ attachment_id,
  sha256, deduplicated, links }`. `entry_id` is optional: a receipt can be
  filed before its entry exists. `file_name` and `mime_type` default to
  the upload's; inline they are required. The upload is taken only after
  the entry and every `link_to` subject check out. `link_to` as in "Tools
  — attachments".
- `books.link_receipt { attachment_id, entry_id }` → `{ ok }`. Links a previously filed
  (unlinked, not withdrawn) receipt to its entry. A receipt linked elsewhere
  must be unlinked first; one already linked to that entry
  (`books.link_attachment`) is refused, as filing it there twice.
  Journaled as `receipt.linked`.
- `books.unlink_receipt { attachment_id, reason }` → `{ ok, entry_id,
  remaining_receipts }`. Detaches a receipt from its entry; it stays filed,
  unlinked. Journaled as `receipt.unlinked` with the reason. Subject to the
  receipt policy: an entry that requires a receipt cannot be left with none
  (`receipt_required` error, remedy `books.attach_receipt`), so replacing a
  receipt is attach-new, then unlink-old.
- `books.withdraw_attachment { attachment_id, reason }` → `{ ok }`. Retires
  a receipt or document that is neither filed on an entry nor linked to
  any subject (take those off first: `books.unlink_receipt`,
  `books.unlink_attachment`); the reason must not be blank. Hidden from listings, never
  linkable again. Journaled as `attachment.withdrawn`; the bytes stay in the
  blob store. Nothing in the evidence trail is ever erased — receipts change
  by new events, the same way entries change by reversals.
- `books.list_receipts { entry_id?, include_withdrawn? }` → metadata list
  (no bytes), each with `withdrawn` / `withdrawn_reason` and `links` (every
  subject it is linked to, its filing entry first). Withdrawn attachments
  are omitted unless `include_withdrawn` is true. `entry_id` lists the
  files filed on the entry and linked to it.
- `books.get_receipt { attachment_id, view?: "meta" | "bytes" }` → `{
  attachment_id, sha256, entry_id?, category, file_name, mime_type,
  size_bytes, description?, links, content_base64?, download? }`. `meta`
  (the default) returns no bytes. `bytes` adds `content_base64`
  (hash-verified) for a file of at most 256 KiB; a larger one gives
  `download`, the command `wanderbooks download --book B <id> --to <a
  folder>` (with `--token NAME` when the caller's token isn't the
  default `agent`; `GET /b/{book}/a/{attachment}`, "Transports"), which
  saves a copy as a new file in that folder. Over stdio there is no
  download: a larger file is refused.
- `books.file_document { category: "formation" | "legal" | "tax" |
  "accounting" | "statement" | "contract" | "photo" | "warranty" |
  "import_source" | "other", file_name?, mime_type?, upload_id? |
  content_base64? + sha256?, description?, link_to? }` → `{ attachment_id,
  sha256, deduplicated, links }`. The bytes, name and type as in
  `books.attach_receipt`. The Drive-folder replacement (today's 01
  Formation / 02 Legal / 03 Tax / 04 Accounting). Never a W-9: it shows
  the full TIN (`books.set_w9 w9_upload_id`).

## Receipt enforcement

Prose tells the model to attach receipts; the policy makes it true. The
engine stays policy-free (it knows nothing of receipts) — enforcement lives
in the MCP layer, which reads the `attachments` table before confirming a
posting.

- **Policy** (the books' settings, "Company settings"; not per-call):
  `policy.evidence.threshold` (receipts strictly above it; unset
  `"25.00"`; `"0.00"`: every entry), `policy.evidence.required_accounts`
  (account codes whose entries always need one, e.g. meals and travel)
  and `policy.evidence.exempt_accounts` (e.g. automated bank fees; an
  entry is exempt only when every line is in one). Changed with
  `books.set_setting`; books from before P1 read `config.json`'s
  `receipt_policy`.
- **Preview.** Every `post_entry` preview includes the receipt checklist
  (`receipts: [...]`, `receipt_required: bool`, `receipt_missing: bool`,
  `split_detected: [...]`), so the human sees it before confirming. A
  `reverse_entry` preview carries none: a reversal needs no receipt (the
  original carries the evidence).
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
  (`asset_id` instead of `entry_id` for a disposal; `split_group: [ids]`
  when the split check triggered.)
- **Not checked yet:** `books.create_invoice`, `books.create_bill`,
  `books.receive_payment`, `books.pay_bill`, `books.run_payroll` and
  `books.remit_payroll_taxes` post at once with no preview and no receipt
  check: attach their evidence right after, to the entry they return.
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
  financing_accounts?, investing_accounts? }` → `{ content,
  content_encoding?, warnings? }`.
  - Reports: `profit_and_loss`, `balance_sheet`, `cash_flow` (direct
    method; needs `cash_accounts`; json is `books.cash_flow`'s answer, csv
    is `section,kind,class,entry,date,label,amount,on_behalf` (`on_behalf`
    on a section's lines only) with sections
    `operating`, `investing`, `financing`, `reconciliation`, `noncash`,
    `footing` and `warning` (its kind, entry and message; no amount); xlsx
    and pdf are the printed statement, operating by class and investing
    and financing by kind with each kind's transactions under it, with its
    two schedules), `trial_balance`, `members_equity` (statement of
    changes in members' equity; members from the settings (`company.members`); earnings
    not yet closed are split the way the next members close will split
    them, so a member's capital on a day never changes when its month is
    closed: rules.md B-13, D-38),
    `general_ledger` (by account: beginning balance, postings with running
    balance and split, ending balance), `general_journal` (alias
    `journal`: each entry posted in the period, line by line; `iif`:
    every posted entry from the first through `as_of`, never a period),
    `tax_summary` (see "Tools — tax categories"), `fixed_asset_register`
    (as of a day; see "Tools — fixed assets").
  - Formats: `pdf` and `xlsx` (base64) for `profit_and_loss`,
    `balance_sheet`, `cash_flow`, `trial_balance`, `members_equity`,
    `tax_summary` and `fixed_asset_register`; `xlsx`, `csv`, `json` for
    every report (`general_ledger` and `general_journal` have no pdf);
    `iif` for `general_journal` (alias `journal`), for QuickBooks Desktop.
  - Dates (rules.md B-30): the period reports (`profit_and_loss`,
    `cash_flow`, `members_equity`, `tax_summary`, `general_ledger`, and
    `general_journal` as csv, json and xlsx) take `start` and `end`
    (omitted: the fiscal year to date; a start after the end is
    refused); `balance_sheet`, `trial_balance`, `fixed_asset_register`
    and `iif` take `as_of` (omitted: today, the company's time zone).
  - IIF (`docs/spec/rules.md` B-34; plan D-36): plain ASCII text, every
    line ending CR LF, inline (no `content_encoding`): every posted entry
    from the first through `as_of`, period-closing entries left out
    (QuickBooks keeps none; imported, a close would zero its period's
    profit), each transaction's DOCNUM its entry id; its BANK accounts are
    `cash_accounts`, else the settings'. `start` or `end` is refused: the
    file is cumulative, so QuickBooks' balances match the books'. A
    character past ASCII is transliterated ("René" "Rene", "北京" "Bei
    Jing", "—" "--"), a double quote is a single one (QuickBooks reads it
    as a text delimiter), a tab or line break is a space, a control
    character, or a character with no transliteration, '?'; a colon in an
    account's name or number '-' (a colon separates levels). Within QuickBooks Desktop's limits (Intuit): an account
    name level past 31 characters is cut there; accounts whose names would
    then match get " (code)" (QuickBooks would merge them); a subaccount
    whose QuickBooks type differs from its parent's is written at the top
    level (QuickBooks gives a subaccount its parent's type); each parent
    is declared before its subaccounts; an account number past 7
    characters, or written like another's, is left out of ACCNUM; a memo
    past 4,095 characters is cut. No line names a customer or vendor (no
    NAME column; see the package's README). `warnings` (absent when
    there are none) says what the file writes differently from the books,
    for the human: each account's name, number and place a line (`account 6010's
    name "Café meals" is written "Cafe meals"`, `account number
    "12345678" is left out: QuickBooks Desktop holds 7 characters at
    most`); the memos grouped, each thing written differently once with
    every entry it is in (`"—" is written "--" in the memos of entries 3,
    5, 9`; a run of changed characters is one thing), then the memos cut,
    so a hundred memos with an em dash make one line. No line holds a full
    TIN: a run of nine digits, or every numeral of a text holding one in
    another script, is masked `•`.
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
  categories, and `settings_toml`, the settings that go with it (tax form,
  members, where closes go) as settings-file TOML for
  `books.import_settings`. A proposal: review with the human, then `books.import_chart_csv`.

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
  move. Refuses an empty code, a code in use, an unassigned control
  role's default code for an account of the wrong type or a header
  (placeholder), and any account an approval rule names (revise or retire
  the rule first). `config_mentions` lists each setting still naming the
  old code (as `books.set_setting` keys, or settings-file labels for books
  whose settings are in a file) and each convention whose template or
  approval rule names it: change each with `books.set_setting` /
  `books.set_convention`. Journaled as `account.renumbered`.
- `books.rename_account { account, name, reason? }` → `{ id, code,
  previous_name, name, notes }` (scope `books:post`, destructive: a
  rename changes the label every past report prints): give an account a
  new name (rules.md B-31). Only the name changes (entries name accounts
  by id): no entry, balance or statement amount moves, and every report,
  past periods included, prints the new name. Refuses an unknown account,
  a blank name, the name it has, a name another account under the same
  parent has (ignoring capitals), one holding a run of nine digits, or
  one that isn't one line of at most 200 characters (kept trimmed); a
  `reason` is one to 2,000 characters with no full TIN. Journaled as
  `account.renamed { account_id, code, name, previous, reason }`, keyed
  by id (a code can change; an id can't), under the journal feature
  `account_renamed` (rules.md B-27; books an older version wrote refuse
  it, `format_upgrade_pending`, until their format upgrade), so an
  earlier version stops at the declaration. `notes` always says that QuickBooks
  matches accounts by name (tell a CPA who imported an earlier IIF), says
  when sub-accounts' full names change with it, when the new name moves
  the account into or out of the cash accounts (`books.cash_accounts`
  unset: picked by name) or the card accounts (named "card", while no
  business-card payment method is recorded), and lists
  conventions whose text names the old name.
- `books.set_tax_category { account, category?, deductible_percent? }`:
  map a revenue or expense account to a category (null clears),
  optionally overriding its deductible percent. Under
  `policy.require_tax_category = refuse` a revenue or expense account
  that isn't a header is never left without one. Journaled as
  `account.tax_set`.
- `books.export { report: "tax_summary" }`: book vs. tax amounts by
  category, the difference where they disagree (meals 50%, penalties 0%),
  and every P&L account still uncategorized. Gains and losses on business
  property (`form_4797_gain_loss`) are listed apart in `form_4797`, at
  their book amounts with no tax amount and outside income and both nets,
  on every return: Form 4797 figures them from the tax basis and splits
  ordinary from section 1231 gain, which decides the line (never Schedule
  C). In the CPA package for the fiscal year to date.
- `books.import_chart_csv` accepts an optional fifth `tax_category` column,
  validated before anything is created. A revenue or expense row without
  one is listed in `warnings`, or, under `policy.require_tax_category =
  refuse`, refuses the whole file (a row is never a header account: the
  file can't say so).
- `policy.require_tax_category` (`books.set_setting`): `warn` (unset) or
  `refuse` (rules.md B-32). The accounts invoices, bills and payroll
  create (Revenue 4000, General Expenses 6999, 6100, 6110) are never
  refused; their results' `warnings` name them. A change naming the key
  needs the journal feature `require_tax_category`: books an older
  version wrote refuse it (`format_upgrade_pending`) until the owner's
  format upgrade (rules.md B-27).

## Tools — import

- `books.import_bank_csv { csv_text? | upload_id?, bank_account_code,
  default_offset_code, rules?, date_order?, source_attachment_id? }` →
  `{ imported: [{ entry_id, date, description, amount, offset_account }],
  count, source_attachment_id, source_linked }`: one draft per
  transaction, all or none, from the QuickBooks bank-upload layout
  (`Date,Description,Amount`, payments negative, or
  `Date,Description,<money out>,<money in>`). `rules` maps description
  substrings (ignoring case; the longest match wins) to account codes,
  else `default_offset_code`. `source_attachment_id` (the export as filed, category
  `import_source`) is checked before any draft and linked to the bank
  account (`source_linked`; false when it was already), never to the
  drafts: a bank export is not a receipt.
- `books.import_chart_csv { csv_text? | upload_id? }` → `{ created,
  count, warnings? }`: creates accounts from
  `code,name,type,parent_code[,tax_category]` CSV. All-or-nothing; the
  name and tax category rules are `books.add_account`'s.
- Each takes the CSV as `csv_text` or as an upload (`books.begin_upload`
  purpose `import`; UTF-8), never both. The CSV itself is not filed: file
  it with `books.file_document` (category `import_source`) for
  `source_attachment_id`.

## Resources (read-only)

Hand-rolled on `ServerHandler` (rmcp 3.x has no resource macros):
`list_resources` advertises, `read_resource` renders.

- `books://start`, `books://settings`, `books://conventions`
- `books://chart`
- `books://reports/trial-balance`, `books://reports/balance-sheet`,
  `books://reports/profit-and-loss`
- `books://digest?since=2026-10-01`; bare (or with an empty `since=`),
  this month so far
- `books://handbook` (the index) and `books://handbook/{section}` (one per
  section, Markdown)
- `books://receipts/{id}` — receipt metadata (as `books.get_receipt`
  view `meta`; no bytes)

Resources exist so read-only clients (dashboards, the viewer,
auditors) never need tool credentials. Tested end-to-end over stdio.

## The agent operating contract

Any AI client driving these tools follows this contract (it will also live
in the server's tool descriptions, so the model reads it at use time):

1. **Show the preview before confirming any posting.** No silent posts, ever.
2. **Echo exact numbers back to the human.** Amounts as written, no rounding
   in prose.
3. **Attach receipts at record time.** A material entry without its source
   document is an incomplete entry. File first, link later if the entry
   isn't ready. Postings strictly above the receipt policy threshold *cannot*
   post without one — the server refuses with `receipt_required`.
4. **When unsure of categorization, draft and ask.** A draft is cheap; a
   wrong posting is a reversal. Never guess-post.
5. **The digest is the receipt for your work.** After a batch of bookkeeping,
   offer the digest; the human reviews, you fix.
6. **Keep the party directory current.** Every counterparty gets a party;
   every entry is tagged with its party; contractors and landlords get W-9
   facts before they are paid, employees W-4 facts before their first
   payroll, and every payroll line names its employee party. Never a full
   TIN or SSN: the engine refuses nine-digit numbers.

## Out of scope for this phase

Bank feeds, payment processing, payroll tax tables and filing (payroll is
accounting only: the caller supplies every figure), tax filing.

## Tools — periods

- `books.create_period { name, start, end }` → `{ period_id }`.
  Dates are YYYY-MM-DD, inclusive.
- `books.close_period { period_id, confirmed? }` → without confirmation:
  `{ status: "preview", period_id, closing_entry: null, lines: [...],
  members }`: the exact closing entry (the close run on a copy of the
  ledger), posting nothing; with `confirmed: true`: `{ status: "closed",
  period_id, closing_entry, lines, members }`, posting it and marking the
  period closed. The entry zeroes revenue and expense balances as of the
  period end; the net goes to `3999` Retained Earnings (created if
  missing), or to `books.close_into` (a single owner's capital), zeroing
  the `books.close_sweep` accounts (draws, dividends) into it too; or,
  with `company.members` in the settings, to each member's capital
  account by share (each member's distribution account zeroed into it), cumulatively (rules.md B-13; D-10): each close gives a member
  their share of everything closed to these members and shares so far, to
  the currency's minor unit (odd units by largest remainder, ties to the
  member listed first when the count began), less what earlier closes gave
  them, so no member is ever a minor unit from their exact share. A close
  may carry a one-time true-up of rounding by closes made under an earlier
  rule, which can run against the period's result by a cent. `members`
  (null for a close not to the members) is `{ since_close, members: [{
  name, capital_account, share, this_close, running, exact }] }`: the
  close the count starts at (the first members close with these members
  and shares; 1 is the books' first close of all), and per member, in the
  settings' order, their capital account's code, share ("0.50"), what this
  close credits them (negative: debits), what the count's closes have
  credited them in all, and their exact share of everything the count
  closed (unrounded, e.g. "0.005"). The preview's `members` is exactly what
  confirming posts. Periods close in order. Closed periods reject new
  entries.
- `books.list_periods {}` → `{ periods: [{ id, name, start, end, status,
  closing_entry, close_rule }] }`. `close_rule` says which rounding rule a
  period closed to the members was closed under:
  `"members_cumulative_since:N"` (the rule since P3, counted from close N),
  `"members_run"` (2026-10-13 to P3), `"members_per_close"` (2026-10-13) or
  `"members_last_absorbs"` (before 2026-10-13); null for an open period or
  one closed to anything else.

## Tools — bank reconciliation

- `books.mark_cleared { entry_id, line_index, cleared }` → `{ ok }`.
  Only posted lines can be marked; the flag is workflow metadata, not
  accounting data (posted entries stay immutable).
- `books.record_statement { attachment_id, account, period_start,
  period_end, opening_balance, closing_balance }` → the statement `{
  attachment_id, file_name, account_code, account_name, period_start,
  period_end, opening_balance, closing_balance, reconciliation_id? }`.
  Records what a filed statement says (balances as printed, in the
  account's normal direction, cents at most) and links it to the account.
  Journaled as `statement.recorded` then `attachment.linked`. Refused for
  an unknown or withdrawn attachment, one already recording a statement
  (not voided), an account that isn't an asset or liability, or a period
  that runs backwards.
- `books.statements { account? }` → `{ statements }`, by statement date:
  those not voided whose file isn't withdrawn (withdrawing a statement's
  file retires it).
- `books.void_statement { attachment_id, reason }` → the statement as it
  was recorded. For a statement recorded wrong: journaled as
  `statement.voided`; it is no longer listed, backs no reconciliation, and
  the same attachment can record it again. Refused while a reconciliation
  not voided rests on it.
- `books.reconcile { statement_attachment? | account, statement_date,
  statement_balance, confirmed? }` → `{ status: "preview" | "reconciled",
  account_code, statement_date, statement_attachment?, book_balance,
  cleared_balance, statement_balance, difference, uncleared_lines,
  cleared_lines, problems, reconciliation? }`, each line `{ entry_id,
  line_index, date, description, side, amount }`. `difference` is cleared
  − statement; zero means reconciled. With `statement_attachment` the
  account, date, and balance are the recorded statement's (any given must
  agree). Without `confirmed: true` it is a preview that records nothing;
  `problems` lists why a confirmation would be refused. Confirmed, it
  needs a recorded statement (its file not withdrawn: refused before
  anything is journaled), a difference of exactly zero, and a date after
  the account's last reconciliation not voided; it then journals
  `reconciliation.completed` (the cleared lines, the statement) and links
  the statement to it, returning `reconciliation`.
- `books.reconciliations { account? }` → `{ reconciliations: [{
  reconciliation_id, account_code, account_name, statement_date,
  statement_balance, statement_attachment, cleared: [{ entry_id,
  line_index }], voided, voided_reason? }] }`, voided ones included.
- `books.void_reconciliation { reconciliation_id, reason }` → the
  reconciliation, voided. Journaled as `reconciliation.voided`; the
  account can be reconciled through that date again.
- `books.mark_cleared` refuses to un-clear a line a reconciliation not
  voided recorded as cleared: void the reconciliation first. Clearing a
  line dated on or before a reconciliation (an outstanding check that
  clears on a later statement) is allowed.

## Tools — A/R and A/P

- `books.add_customer { name }` → `{ id, party_id }`.
- `books.add_vendor { name }` → `{ id, party_id }`. Both link the record to
  a party ("Tools — parties").
- `books.create_invoice { party_id, date, due_date, lines: [{ description,
  amount }] }` → `{ id, total, entry_id, warnings? }`. Auto-posts Dr
  `1200` Accounts Receivable / Cr `4000` Revenue. `warnings` names a
  revenue account it created with no tax category (rules.md B-32).
  `party_id` is the customer's `id` from `books.add_customer`, not its
  `party_id`.
- `books.create_bill { party_id, date, due_date, lines }` → `{ id, total,
  entry_id, warnings? }`. Auto-posts Dr `6999` General Expenses / Cr
  `2000` Accounts Payable; `warnings` as for invoices. `party_id` is the
  vendor's `id` from `books.add_vendor`, not its `party_id`.
- `books.receive_payment { document_id, date, amount, cash_account }` →
  `{ entry_id, remaining, status }`. Posts Dr Cash / Cr A/R.
- `books.pay_bill { document_id, date, amount, cash_account }` →
  `{ entry_id, remaining, status }`. Posts Dr A/P / Cr Cash.
- `books.ar_aging { as_of? }` → `{ as_of, rows: [{ name, current, days_30,
  days_60, days_90, days_90_plus, total }] }` (`as_of` omitted: today).
- `books.ap_aging { as_of? }` → same, per vendor (`as_of` omitted: today).

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

- `books.run_payroll { date, cash_account, lines: [{ employee?,
  employee_name?, gross_pay, federal_withholding, state_withholding, ss_employee,
  medicare_employee, ss_employer, medicare_employer, other_deductions }] }`
  → `{ run_id, entry_id, total_gross, total_net, warnings? }` (`warnings`
  names an expense account it created with no tax category). One
  balanced entry:
  Dr 6100 Salary & Wage Expense (gross), Dr 6110 Payroll Tax Expense
  (employer SS + Medicare), Cr 2110–2150 withholding payables, Cr cash
  (net). Auto-creates the 61xx/21xx accounts. Respects closed periods.
  `employee` names the employee's party (id or name; it must have the
  employee role, else nothing is written); new runs should name it, and
  `employee_name` then defaults to the party's name. A line needs one or
  the other. When every line with net pay names its party, the entry
  names those parties, each with its net pay, so each employee's
  `books.party` counts it; under `[books] parties = "require"` a line
  with net pay and no party is refused before anything posts
  (`party_required`).
- `books.remit_payroll_taxes { date, cash_account, agency? }` → `{
  entry_id, total }`. Clears every payroll payable to cash in one entry,
  naming `agency` (the tax agency's party) on it; required under
  `[books] parties = "require"` (`party_required`, nothing posted).
- `books.list_payroll_runs` → `{ runs: [{ id, date, employee_count,
  total_gross, entry_id }] }`.

## Tools — projects (classes)

A project is a class with its facts (`docs/spec/rules.md` B-37): the
same id, the same name, so `books.add_line`'s `class`, the P&L `class`
filter and `books.profit_and_loss_by_class` take projects. A `project`
reference is an id or a name (ignoring capitals in any script and
surrounding space; in a link's subject a JSON number is always an id); a
number that is one project's id and another's name, which only old books
can hold, is refused, naming what to pass instead. Names are unique,
ignoring capitals and surrounding space, and never digits only (they
would read as an id). A change that keeps a project's own name never
checks it again, so old books holding a duplicate name can change either
project's facts; a rename onto another project's name is refused. Every project write but a name alone needs the journal feature
`projects` (B-27): books waiting for their format upgrade (D-39) can add
a project by name only, and every other project write (facts, a link to
a project, a decision naming one) is refused with `format_upgrade_pending`
until the owner upgrades them.

- `books.add_project { name, description?, status?, kind?, party?,
  started_on?, ended_on?, links?, notes? }` (books:write) → the project: `{ id,
  name, status, kind, description, party: { party_id, label }?,
  started_on, ended_on, links, notes }`. `status`: `active` (default),
  `paused`, `finished`, `abandoned`. `kind`: `internal` (the company's own
  product), `client` (work for a customer), `on_spec` (work done hoping
  to be paid), `other`; null when not recorded. `party`: who it is for or
  with (id or name). `ended_on` only when finished or abandoned, never
  before `started_on`. `links`: up to 8 web addresses, one line each, no
  spaces, none twice (never opened). Text fields: one line (name, links,
  at most 200 characters) or long text (description, notes, at most
  2,000), never a full TIN. A name alone journals `class.added`; facts add
  `project.set` (the whole record).
- `books.update_project { project, name?, description?, status?, kind?,
  party?, started_on?, ended_on?, links?, notes? }` (books:write;
  idempotentHint) → the
  project. Only the fields given change; `""` clears description, kind,
  party, started_on, ended_on or notes; `links` replaces the list (`[]`
  clears it). At least one field. Every field is checked before anything
  is written; then the whole record is journaled as `project.set`, and a
  record with nothing new journals nothing. A rename changes the label
  every report prints, nothing else (not destructive, as a party's name).
- `books.projects { status? }` (books:read) → `{ through, projects: [{ ...project,
  to_date: { through, revenue, expenses, net, unrecovered_cost } }] }`: every
  project (every class), by name ignoring capitals; `to_date` counts every
  posted revenue and expense line tagged to it through today in the
  company's time zone, closing entries left out; `unrecovered_cost` only
  for an on-spec project (else null): expenses less revenue, never below
  zero.
- `books.project { project, start?, end? }` (books:read; omitted dates: the fiscal
  year to date, B-30, as every period report) → `{ project, period: {
  start, end, revenue, expenses, net, by_account: [{ code, name, type
  ("revenue" | "expense"), amount }], by_month: [{ month ("YYYY-MM"),
  start, end, revenue, expenses, net }] }, to_date (as above, through
  end), entries: [{ entry_id, date, description, status, project_amount,
  total, reversal_of?, receipts: [attachment_id] }], documents: [as
  books.list_receipts], decisions: [as books.decisions] }`. The figures
  are `ledger_core::project_figures`'s, the viewer's: only revenue and
  expense lines count (a tagged asset purchase is listed with its entries,
  its project amount zero); `by_month` holds the period's last 240 months
  at most. `to_date` always counts every tagged line through `end`,
  whatever the project's dates: job-to-date beside the period. The on-spec
  `unrecovered_cost` is the "if repaid" figure, never a receivable (ASC
  450-30-25-1). Entries are newest first, drafts included; decisions
  oldest first, superseded ones too.
- `books.add_class { name }` → `{ id }`: a project with only its name;
  refuses a name another project has.
- `books.list_classes` → `{ classes: [{ id, name }] }` in id order.
- `books.add_line` takes optional `class` (a project: name or id): tags the
  new line. Shared costs are split explicitly across lines at entry time.
  Its result gains `note` when the project is finished or abandoned
  ("project X is finished (ended 2026-09-30): tag a late cost or invoice
  to it only if it belongs to it"): tagged anyway.
- `books.profit_and_loss` takes optional `class` (a project: name or id)
  to filter to.
- `books.record_payout` lines and adjustments take optional `class` (a
  project); its result gains `notes`, one per finished or abandoned
  project tagged, as `books.add_line`'s `note`.
- An invoice or a bill takes no project yet: `books.create_invoice` and
  `books.create_bill` post their entry at once with untagged lines, and
  only a draft's lines take a class (`books.add_line`'s `class`). Link the document to the
  project (`books.link_attachment` subject kind `project`); when its lines
  must count in the project's figures, book it as a hand-built entry
  (`books.draft_entry` + `books.add_line` class) instead, which then is
  no invoice or bill in the agings (an open item: tagged document
  lines).
- `books.profit_and_loss_by_class { start?, end?, class? }` (omitted dates:
  the fiscal year to date, as every period report) → per-class revenue,
  expenses, net income, plus an unclassified row (only that class's row
  when `class` is given; an unknown or ambiguous class is refused). Reversals carry their
  class so divisional P&L nets correctly.

## Tools — CPA handoff

- `books.export_cpa_package { as_of?, cash_accounts?, financing_accounts?,
  investing_accounts?, include_receipts?, include_tins? }` → `{ file,
  tins_included, tin_gaps, warnings?, note }`: the zip,
  `cpa-package-<as_of>.zip`, handed over as `file` (a `HandedFile`,
  "Tools — the TIN vault").
  **As of one day** (`docs/spec/rules.md` B-33): nothing in it reflects
  anything dated after `as_of` (default today): no figure, entry,
  receipt or worksheet row; the chart and `documents/` are as they stand.
  One list of cash accounts and one of financing accounts serve the
  whole package: the request's, else the settings'.
  **QuickBooks file** (`general-journal.iif`, B-34): as `books.export`
  iif makes it, through `as_of`; `warnings` says what it writes
  differently from the books, in the same lines as `books.export` iif's
  (account names and numbers one each, the memos grouped; every other
  file keeps the books' text), the README lists the same lines (and
  names the entries with a line on accounts receivable or payable, to
  which QuickBooks Desktop may ask the CPA to add a customer or vendor),
  and, when the file writes anything differently, `note` ends "N account name(s), number(s), place(s) or memo(s)
  are written differently in general-journal.iif to fit QuickBooks
  Desktop (warnings says how; so does the README): tell the human.", N
  counting each name, number, place and memo.
  **Receipts** (`include_receipts`, default true): `receipts/` holds every
  current file behind each posted entry of the fiscal year to date, filed
  on it or linked to it (`books.link_attachment`; a linked file counts as
  the entry's receipt), one copy per entry, each with a row in
  `receipts/index.csv`; `documents/<category>/` holds the files behind no
  entry, receipts excepted.
  **TINs** (`include_tins`, default true; DECISION, the owner, 2026-10-15):
  the K-1 worksheet and the 1099 worksheet carry every member's and
  payee's full TIN from the vault; the zip, and each file in it with a
  TIN, is recorded so that, given back to be filed, it is refused. A
  locked vault or a missing TIN never fails the
  package: the row says why ("TIN not in the vault", "the TIN vault is
  locked", a record that disagrees with the books), the README lists it,
  and `tin_gaps` names each member and each reportable payee without one.
  `include_tins: false`: last fours only. `note` is "The package, with
  last fours only.", or "The package, with full TINs, for the CPA.",
  followed, with gaps, by "N TIN(s) are missing (tin_gaps): collect them
  with books.set_tin or books.set_w9 and export again." **K-1 worksheet** (`k1-members.csv`, with
  `[[company.members]]`): one row per member, in order (with more than one
  member, a last row totals them, "Total, all members (not a K-1)", to tie
  to Schedule L; a worksheet that can't be made is replaced by
  `<name>-NOT-INCLUDED.txt` saying why, the K-1, tax-basis and 1099
  worksheets alike): `member,
  party_id, share, capital_account, tin, tin_type, tin_note` (without
  TINs: `tin_last4, tin_type`), then `beginning_capital,
  contributions_cash, contributions_property, net_income, distributions,
  other, ending_capital` for the fiscal year to date: the statement of
  members' equity's figures (`ledger_core::k1`), contributions split into
  property (what an entry crediting a member's capital debits to asset
  accounts other than cash and receivables: equipment, software, or a
  prepaid service contract the members owned and handed over straight to
  capital, e.g. AppleCare+ contributed with a laptop) and cash (the rest:
  deposits, and what the members paid for the company, including a
  service they paid a vendor ahead for, booked through Due to
  Shareholders and then converted to capital; DECISION, the owner,
  2026-10-15). Property in an entry crediting several members is shared
  in proportion to their credits. An entry and its reversal both in the
  year cancel: neither is a contribution or a distribution. A member's
  party is the party with the member's name (not a display name) and the
  member role, as the viewer's capital statement finds it; with none, the
  member's TIN is a gap. **Tax-basis capital** (`k1-tax-basis-capital.csv`,
  P3, D-41, rules.md B-36; Form 1065 instructions, Item L, the tax-basis
  method): per member, `member, item, book, tax_basis, book_less_tax,
  status, note` for each Item L line (beginning; capital contributed,
  cash and property; current year net income (loss); other increase
  (decrease); withdrawals and distributions; ending), the property at its
  §723 basis (`ledger_core::tax_capital`), then the reconciliation of book
  to tax capital by cause (the §704(c) built-in gain or loss share,
  property without a tax basis in the books, property distributed). A
  line the books can't know (tax depreciation of the fixed assets in
  service, a disposal's tax gain, the basis of property distributed or of
  contributed property no asset holds) has `tax_basis` blank, status
  `open: the CPA's`, and a note with what the books know so far and the
  book amount the CPA's figure replaces; never a guessed number.
  **1099 worksheet** (`1099-worksheet-<year>.csv`, `as_of`'s calendar
  year) is `books.vendor_1099`'s CSV (above) for the package's cash and
  financing accounts: the payments from January 1 through `as_of` (the
  year to date; the whole year for a December 31 package; DECISION D-36),
  and the README says which. `books.vendor_1099` given the same
  `cash_accounts`, `financing_accounts` and `through` hands over the
  identical file. With TINs it adds `tin, tin_type, tin_note`. A zip with
  the full set, each
  statement as PDF and XLSX: `profit-and-loss`, `balance-sheet`,
  `cash-flow` (the direct method, as `books.export` prints it: operating
  by class, investing and financing by kind with each member named, and
  its two schedules; reconciled to the package's cash accounts),
  `members-equity`
  (multi-member LLCs), `tax-summary`, `trial-balance`;
  `fixed-asset-register` (PDF and XLSX) when an asset was acquired by
  `as_of`; plus `general-ledger.xlsx` and `general-journal.xlsx`; and
  `README.txt` first (the year, the basis, the journal's verified event
  count and what each file is). Period reports cover the fiscal year
  holding `as_of`, through it.
  What the CPA needs; they never log into the system.
