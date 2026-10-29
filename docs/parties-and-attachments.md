# Parties and attachments

Design for two features built together (the owner, 2026-10-15): a directory
of the parties the company deals with, and attachments that link to any
subject. Status: built (2026-10-15 to 2026-10-17: parties with roles, the
TIN vault, attachments linked to any subject; the party's contact block,
D-55, is built on branch S4.4 as of 2026-10-28). The design is kept as
written; departures are in `plan/served.md` and `docs/spec/rules.md` B-26.

## Why together

Both answer "what is this connected to?" A W-9 note, a contract, a bank
statement, a photo of a laptop: each is a file about a party, an account, a
reconciliation, an asset, a period. One subject model serves both.

## Parties

A party is anyone the company deals with. Today's vendors and customers
become parties; employees stop being name strings on payroll lines; the
Apple, Google, Anthropic, IRS, and Utah that live only in entry
descriptions get records.

### Model (ledger-core, `parties.rs`)

```text
Party {
  id: PartyId,                 // its own id space (vendor and customer records keep theirs)
  name: String,                // legal or trade name, as on documents
  display_name: Option<String>,
  roles: BTreeSet<Role>,       // set by hand: contractor, employee, member, payout_platform,
                               // tax_agency, government_agency, financial_institution,
                               // landlord (vendor and
                               // customer follow the records; see all_roles)
  website: Option<String>,
  address: Option<String>,     // business address; no personal PII beyond it
  notes: Option<String>,
  archived: bool,              // no longer dealt with; history stays
  w9: Option<W9Facts>,         // contractor/landlord: the TIN itself is in the vault
  w4: Option<W4Facts>,         // employee: never the SSN
  vendor: Option<VendorId>,    // its vendor record (bills, payments, 1099 form)
  customer: Option<CustomerId>,// its customer record (invoices)
}
W9Facts { received: NaiveDate, entity_type: String, exempt_payee_code: Option<String>,
          fatca_code: Option<String>, tin_last4: String, kept_at: String,
          vault_record: Option<String> /* SHA-256 of the vault record */ }
W4Facts { received: NaiveDate, state: String, kept_at: String }
```

The 1099 form stays on the vendor record (`vendor.1099_set`); a party
reaches it through `vendor`.

Rules (to vendor as B-26 with sources):

- A full TIN or SSN is never in the journal: `tin_last4` is exactly four
  digits, and any nine-digit run in any party field is refused (the engine
  checks, not the agent). Full TINs live only in the vault (below).
  Source: IRS truncated-TIN rules (Treas. Reg. §301.6109-4, to vendor) for
  what may appear; DECISION (the owner, 2026-10-15) for the vault.
- Roles are a set; each role's facts are only allowed with the role
  (W-9 facts need contractor or landlord; W-4 facts need employee).
- Names are unique ignoring case and surrounding space among
  non-archived parties.
- A 1099 needs a contractor or landlord role; 1099 totals come from the
  entries tagged with the party, as today's vendor 1099 report does.

### Links

- Entries: `entry.party_set {entry_id, party_id|null}` generalizes
  `entry.vendor_set` (one counterparty); `entry.parties_set` names
  several, each with a share (below).
- Invoices and bills already name a customer or vendor id: those are
  party ids after migration.
- Payroll lines name an employee party id (old runs keep their name
  strings; new runs must name a party with the employee role).

### Journal events

`party.added {id, name, roles}`, `party.updated {id, change}` (one
`PartyField` per event, `{field, value}`: name, display_name, website,
address, notes, archived), `party.role_set {id, role, on}`,
`party.w9_set {id, facts|null}`, `party.w4_set {id, facts|null}`,
`party.tin_set {id, facts|null}` (TIN facts: type, last four, vault
record, when purged), `party.creditor_set {id, account_id, on}` (P3: the
party is, or stops being, the creditor of a liability the company owes it
itself; the journal declares its feature, `party_creditors`, before the
first one), `entry.party_set {entry_id, party_id|null}`,
`entry.parties_set {entry_id, parties: [{party_id, share}]}` (a share is
`{"amount": "30.00"}`, `{"percent": "60"}`, or `"derived"`; `[]`
clears). Replayed and projected like
the rest; every event gets viewer journal text (the GUI test enforces it).

### Shares: an entry with several parties

DECISION (the owner, 2026-10-15): there is no "joint" party. A joint
transaction is with both members and shows on both members' pages, each
with their share; apportioning is explicit and exact when it isn't
derived. Spec: `ledger_core::shares`.

- An entry names a list of `PartyShare {party, share}`; `Share` is
  `Amount(d)`, `Percent(p)`, or `Derived`. Tagging one party
  (`set_entry_party`) is one derived share: all of it.
  `Ledger::entry_party` means "the sole party"; `entry_parties` lists
  them all.
- **What a share is of** (rules.md B-35, P3 defect 14): one signed
  target, debit-positive as a line on the parties' own account (positive:
  the company paid or gave; negative: it received), with its direction
  (paid, received, neither) and what of it was money and what property in
  kind. A close: its parties' own-account movement, neither. A reversal:
  its original's target negated, never re-read from its own lines. An
  entry naming only members that moves equity: what moved their capital.
  A contribution is received, the property in it in kind (the K-1
  worksheet's test), the rest cash; a distribution is paid, the money it
  paid cash, the property it gave in kind, the rest (declared to
  distributions payable, or credited to what the members advanced once
  they are named its creditors) still owed, counted when paid. Money moved: the money
  (`parties::flow`), all cash. Otherwise its parties' own-account
  movement (a bill −, an invoice +), else its total debits: neither,
  counted in no total. Read from journaled facts only (the member role,
  the chart, `creditor_of`), so leaving `[[company.members]]` never
  changes a posted entry's reading. (Before P3 a property contribution
  read +500.00, the sign of a payment, and its reversal the same.)
- **Amount:** exactly that, positive, in cents; the amounts must sum to
  what the entry moved.
- **Percent:** more than 0, at most 100, summing to exactly 100. Each part
  is rounded toward zero to the cent and the leftover cents (fewer than
  one per party) go to the first party in order.
- **Derived:** a sole party gets all of it. Several parties must all be
  members (`[[company.members]]` matched by name to a party with the
  member role; core is handed them as `PartyMember`s). If the entry has
  lines on a named member's capital or distributions account, each
  member gets what their own lines say (and those must account for all
  it moved); otherwise each gets their income share, rescaled to the
  members named, rounded as percents are.
- **Posted is forever, the split included:** when an entry naming
  several parties with derived shares posts (or a posted entry is tagged
  so), the server records the exact amounts they resolve to
  (`entry.parties_set` with amounts), so a later change to
  `[[company.members]]` (a new income split, a member archived) never
  re-splits posted history. A draft's derived shares follow the settings
  until it posts. An archived member stays a member for their history
  (`Company::party_members` falls back to the archived party of that
  name). Entries posted with derived shares before this rule keep them
  until re-tagged with `books.set_entry_party`.
- **Refused** (journaling nothing): a party named twice, an unknown
  party, mixed kinds of share, percents not summing to 100, an amount
  finer than a cent, and two parties with vendor records. A 1099 counts
  a payment toward a single vendor: the vendor tag follows the one party
  with a vendor record, and the 1099 worksheet counts the whole payment
  toward it, so name a vendor among others only when the whole payment
  was its (otherwise record its part as its own entry). Whether shares
  add up depends on the settings (cash accounts, members), so core
  checks only their form; the MCP server checks they resolve once the
  entry has balanced lines, and `post_entry` refuses (`party_shares`)
  until they do. Replay never consults settings.
- **Money:** `parties::activity_with` counts each party's part on two
  bases (B-35 item 4): cash (its money: cash, a card, the members'
  financing, a loan) and all (adding property contributed or distributed
  in kind), the viewer labelling the first "cash basis". Money accounts
  are the cash accounts, the cards and `[books] financing_accounts`; any
  other liability is a promise, counted once, when settled (P3 defect
  24). A party may be named the creditor of liabilities it is owed itself
  (`creditor_of`: a card's issuer, a loan's lender, each member for what
  the members advanced); a credit to one never counts as paying it. An
  entry that counts nothing (a bill, an invoice, a close) is never
  unresolved; an entry whose shares don't resolve counts for no one
  (listed as unresolved). Reversals carry every share, so a reversal
  subtracts each party's part; re-tagging either one (parties, shares,
  or vendor) re-tags the other, so the pair always nets to zero.
  A reversal counts on its root original's side, so reversing the
  reversal reinstates the payment as paid (never a negative received).
  For a member, basis all over their equity entries is the statement of
  members' equity; basis cash their contributions is the K-1 worksheet's
  cash contributions.
- **One number everywhere:** `books.party`, `books.parties`, and the
  viewer's party page and directory read money as the server does
  (`[books] cash_accounts`, else every asset account but receivables;
  the cards; `[books] financing_accounts`) and count the whole calendar
  year, so a posted entry dated later this year counts in both. The
  directory's TIN last four is the TIN facts', else the W-9 facts'
  (`Party::tin_last4`, P3 defect 11), as everywhere the books show one. The viewer shows the year up to the later of today
  and its last posted entry.
- **For the agent:** a joint member transaction (a reimbursement on Due
  to Shareholders, a contribution or distribution from joint funds)
  names both members with no shares and lets them derive; an uneven
  split gives amounts.

### A member's capital statement

A member's party page (role member, named in `[[company.members]]`,
whose capital account is in the chart) is their capital account
statement over the page's period (`Books::capital_statement`). Every
figure is the CPA package's K-1 worksheet row for that member and span
(`ledger_core::k1::k1_detail`, itself the statement of changes in
members' equity split into cash and property), so the page and the
K-1 never disagree:

- capital at the start of the period (including their share of earnings
  not yet closed into capital);
- add contributions of cash (deposits, and what they paid for the
  company) and of property (debits to asset accounts that are neither
  cash nor receivables, at the value booked: fair market value);
- add their share of the period's net income, closed or not;
- less distributions;
- any other change in their capital, when there is one;
- capital at the end of the period (always opening plus the movements,
  exactly);
- their share of profit, loss, and capital; and their part of what the
  company owes the members jointly, split by the members' shares: the
  balance of the `[books] financing_accounts` liabilities (the
  liabilities members advance), or, only when that is not set, of the
  liability accounts named "due to" / "owed to" (such as 2100 Due to
  Shareholders).

Cash and property are told apart by `[books] cash_accounts`. Without it
the statement reads money as the engine does (every asset account but
receivables), so every contribution reads as cash; set `cash_accounts`
to see property apart. (The Cash page keeps its own name-based guess.)

### Migration

Old journals replay unchanged. `vendor.added` and `customer.added` keep
their meaning and, on replay as live, link their record to a party: the
live party of that name (ignoring case and surrounding space) without
such a record yet, else a new party. So a vendor and a customer of the
same name share one party; a second vendor of a taken name gets a new
party named "Name (2)" (names stay unique). Vendor and customer names
obey the party text rules (no full TIN). `entry.vendor_set` also sets the
entry's party, and `entry.party_set` to a party with a vendor record sets
the vendor tag: one counterparty. On an entry naming several parties the
vendor tag follows the parties, never the other way: `entry.vendor_set`
for the vendor of a party already named keeps every share; for any other
vendor, or clearing the tag while its party is named, it is refused. A
vendor record can't link to a party named beside another party with a
vendor record (refused: the entry would name two vendors); linking one
tags the entries naming the party that have no vendor tag yet.

### Tools

`books.add_party`, `books.update_party`, `books.party_roles`,
`books.set_w9`, `books.set_w4`, `books.parties` (list, filter by role),
`books.party` (one: facts, roles, totals this year, documents, links),
`books.set_entry_party`. (Merging two parties, `books.merge_parties`, is
not built yet: records of one name already share a party.) `books.add_vendor` and
`books.add_customer` stay as shortcuts.

### Viewer

A Parties page (by role, searchable) and a page per party: facts, roles,
year-to-date paid / owed / invoiced / 1099 amount, every entry, invoice,
bill, payroll run, documents, and history. Members link to their capital
accounts. Global search finds parties. Needs-attention: a contractor
paid without W-9 facts; a 1099 party paid over the threshold.

Money with a party (the owner, 2026-10-15: "how much have I spent on Apple
versus how much did I get back"): paid out, received in, and net, for a
chosen period (this year, last year, any year, all time), from
`ledger_core::parties::activity_with`, labelled cash basis, with "With
property contributed or distributed: paid $X, received $Y" under them
when anything moved in kind; what it was for (the non-money lines by
account); a monthly out-vs-in chart; the 1099 amount for the calendar
year. An entry's page says each party's part in words ("received
$740.00, in kind"; "paid $1,480.00 (reversal of #12)"). The directory
shows the same columns, sortable.

## The TIN vault

Decision (the owner, 2026-10-15): wanderbooks holds full TINs, so what it
hands the CPA is complete, but never in the journal. The vault holds
everyone's, members included, and the CPA package carries them: "In the
vault, that's the WHOLE reason we built it. And it needs to be in the
damn CPA package."

- **Where:** `vault/` in the books dir, listed in `.gitignore`. Never in
  the hash-chained journal, the SQLite projection, git, an export other
  than the 1099 worksheet and the CPA package (handed to the agent as a
  file to download, never saved in the books), or anything the viewer
  draws. Nothing outside the vault holds
  more than the last four digits.
- **Who:** any party, whatever its roles: every member (their K-1 needs
  it) and every 1099 payee. The books keep the party's TIN facts (type,
  last four, the vault record's hash; `party.tin_set`), which never
  disagree with its W-9 facts' last four. The viewer shows "TIN on file
  ••••1234 (SSN)" on the party's page, never more; clicking it copies the
  full number (right-click: digits only), read from the vault at that
  moment with the key in force, so people can paste it into a form
  (the owner, 2026-10-16: "people still need to actually access these
  things"). The viewer never creates a key: without one it says the vault
  is locked.
- **What:** per party, the full TIN, its type (SSN, EIN, ITIN), and the
  W-9 file itself, each encrypted at rest (XChaCha20-Poly1305, one random
  nonce per record; the file format carries a version and the party id as
  associated data, so a record can't be moved to another party).
- **Key:** a random 256-bit key in a key file outside the books dir
  (default `~/.config/wanderbooks/vault.key`, mode 0600; `[vault] key_file`
  in wanderbooks.toml overrides it), created on first use. No OS keyring
  or other system service (the owner, 2026-10-15: no external dependencies),
  so the vault works anywhere wanderbooks runs. Recovery is a copy of that
  file in the human's password manager; without it the vault can't be
  read (the books still open and verify).
- **Journal:** `party.w9_set` records that a W-9 was received, its date,
  entity type, codes, and last four, plus a hash of the vault record, so
  the books prove a W-9 existed without holding it.
- **Purge:** `books.purge_tin {party, reason}` deletes the vault record
  (after the retention period, or on request) and journals that it was
  purged; the journal's facts stay.
- **Use:** `books.vendor_1099` with `include_tins: true` and
  `books.export_cpa_package` (its K-1 worksheet, `k1-members.csv`, and its
  1099 worksheet; `include_tins` defaults to true there) decrypt them.
  The package is handed over as a file (an export the asking token
  fetches with `wanderbooks download`); no tool takes a path. The MCP server
  never returns a TIN in any other tool, log, or error. Tests check that
  no other output contains a nine-digit run.
- **Books without a vault** (another machine, books adopted without
  theirs; a restored backup brings its vault and key, D-31) still
  open and verify; the 1099 worksheet and the CPA package then say which
  TINs are missing (`tin_gaps`), and the package is still built.
- **Tools** (built; `docs/mcp-interface.md` "Tools — the TIN vault"):
  `books.set_w9` takes `tin` + `tin_type` and the signed form as
  `w9_upload_id` (`books.begin_upload`; never inline, D-46) (the form goes to the vault, never the attachment store) and journals the last
  four with `vault_record`; facts that disagree with the vault's TIN are
  refused, and clearing facts with a vault record needs a purge first.
  With a TIN it also records the party's TIN facts. `books.set_tin
  {party, tin, tin_type}` stores any party's TIN (a member's SSN, say) the
  same way and journals `party.tin_set`; a party whose W-9 facts name
  another last four is refused (a payee's new TIN comes with its W-9).
  `books.get_w9 {party}` hands the form over the same way.
  `books.purge_tin {party, reason,
  confirmed?}` previews, then deletes the records (no key needed) and
  journals the purge as `party.w9_set` with `vault_record` cleared and
  `kept_at` saying when and why, and `party.tin_set` with `vault_record`
  cleared and `purged` saying when and why. `books.vendor_1099 {include_tins: true}`
  is the one tool output with full TINs (with a warning; `csv: true`
  also hands the worksheet over as a CSV file);
  `books.export_cpa_package` carries them in its zip, handed over the
  same way. The key is the book's own (`keys/legacy.key` in its folder in
  the data root). `tests/spec_vault.rs` (ledger-cli) scans every tool
  response, every file handed over, the server's log, and every file in
  the books' folder outside `vault/` (and the files handed over, kept for
  their download) for the TINs in any formatting.

## Attachments

### Subjects

An attachment links to any number of subjects:

```text
Subject = Entry(id) | Account(id) | Reconciliation(id) | Asset(id) | Party(id)
        | Invoice(id) | Bill(id) | Period(id) | PayrollRun(id) | Schedule(id)
        | Company
```

`attachment.linked {attachment_id, subject}` and `attachment.unlinked
{attachment_id, subject, reason}` replace the single `entry_id` link (old
`receipt.attached` with an entry and `receipt.linked` replay as links to
that entry). A receipt is an attachment linked to an entry; the receipt
policy counts links to the entry, as now. A file stored once can be
linked to many subjects (no more refiling).

Tools (built; `docs/mcp-interface.md` "Tools — attachments"):
`books.link_attachment`, `books.unlink_attachment` (refused for a posted
entry's last receipt under the receipt policy, and for a standing
reconciliation's statement), `books.attachments {subject}`, `link_to` on
`books.attach_receipt` and `books.file_document` (all or nothing), and
`links` on every listed attachment. `books.file_document` takes the kinds
below as its category (`receipt` is `books.attach_receipt`'s).

### Kinds

`receipt`, `statement`, `contract`, `formation`, `legal`, `tax`,
`accounting`, `photo`, `warranty`, `import_source`, `other`. A statement
carries `{account_id, period_start, period_end, opening_balance,
closing_balance}`, checked against the account's currency.

### Reconciliations

`books.reconcile` with `confirmed: true` records a reconciliation:
`reconciliation.completed {id, account_id, statement_date,
statement_balance, cleared: [(entry_id, line)], attachment_id}`. It
refuses unless the difference is exactly zero and a statement for that
account and date is attached (the receipt rule's twin). A reconciliation
is never edited; a later one supersedes it; `reconciliation.voided
{id, reason}` exists for mistakes. While it stands, the lines it
recorded as cleared can't be un-cleared (void it first), and nothing is
journaled when it is refused (a withdrawn statement file included). A
statement recorded wrong is voided (`statement.voided {attachment_id,
reason}`, refused while a reconciliation rests on it) and recorded again
on the same file; withdrawing a statement's file retires its statement. Viewer: per-account reconciliation
history with its statement; Needs-attention: an account with activity not
reconciled through last month's end.

Tools (built; `docs/mcp-interface.md` "Tools — bank reconciliation"):
`books.record_statement`, `books.statements`, `books.reconcile` (preview
by default; `statement_attachment` + `confirmed: true` records it),
`books.reconciliations`, `books.void_reconciliation`,
`books.void_statement`; `books.show
reconciliation`; `books.digest` `unreconciled_accounts` and the
viewer's Needs attention, one rule (`ledger_store::reconcilable`): the
cash accounts (`[books] cash_accounts`, else assets named for cash), the
card accounts (`ledger_core::parties::card_accounts`: the business-card
payment methods' liabilities, else the liabilities named "card", not
"gift", before their first statement),
and other liabilities with statements, not reconciled through last
month's end). The statement of a reconciliation not voided can't be
unlinked from it.

### Import sources

Built: file the export (`books.file_document` category `import_source`)
and pass `source_attachment_id` to `books.import_bank_csv`; it is linked
to the bank account imported into. Not to the drafts it creates: a file
linked to an entry counts as its receipt, and a bank export is not one
(DECISION, 2026-10-15, building the MCP tools). Recording the row each
draft came from is not built yet.

### Projection

SQLite: `attachment_links(attachment_id, subject_kind, subject_id)`,
`statements(attachment_id, account_id, period_start, period_end, opening,
closing)`, `reconciliations(...)`, `reconciliation_lines(...)`; the old
`attachments.entry_id` column is kept and filled for entry links during
the migration so older readers still work.

## Testing

Spec blocks on every command; spec-blind suites per crate; the
refusal-leaves-books-unchanged property; replay and migration from v10
books; the GUI's every-event-has-text test; mutation on the new modules.

## After this: the completeness engine

Decided (the owner, 2026-10-15), built after parties and attachments: one
set of completeness rules in the core (asset: tag, serial, tax basis,
acquisition link, label; entry: receipt, 1099 vendor tag; account:
reconciled through last month; month: depreciation, amortization, close;
party: W-9 before paying a contractor; settings: no problems; and the
rules this design adds), `next_steps` on every tool response naming the
exact call that completes each, `books.todo` as the one punch list behind
the digest, the viewer's Needs attention, and the session-start hook, and
enforcement where a rule is a rule (tax basis at registration). The
standard: do it now, completely.

### Counterparties

Decided (the owner, 2026-10-15): the books should be easy to make complete
without forcing one style. `[books] parties = "off" | "suggest" |
"require"`, default `"suggest"`:

- `off`: never mention counterparties.
- `suggest`: a draft or preview without a party names the likely one
  (existing parties matched against the description, the receipt's text,
  the vendor), or offers to create one; `next_steps` carries the exact
  call. Nothing is refused.
- `require`: posting is refused until the entry names a party or is
  marked as having none (transfers, depreciation, amortization, closes
  are exempt by kind).
  The engine's own posting paths name their counterparty or are refused
  first, so nothing posts around the rule: `books.run_payroll` names each
  line's employee party on the entry (each with their net pay) and, under
  `require`, refuses a line without one; `books.remit_payroll_taxes`
  takes `agency` (required under `require`); `books.dispose_asset` takes
  `buyer` (required under `require` for a sale with proceeds; a
  retirement with no proceeds and no buyer is marked as having none).
- **Marks follow the chain:** a reversal carries its original's parties,
  shares, vendor tag, and no-party mark; marking or tagging any entry of a
  reversal chain marks or tags them all. Tagging a vendor (or any party)
  clears a no-party mark, so an entry never names a party and carries the
  mark at once.

Also: memorized matches (journaled rules like description contains
"OPENROUTER" → OpenRouter, applied by bank-CSV import and new drafts,
reviewable and removable); a backfill punch list grouping untagged
entries by likely party for one confirmation per group; 1099 nudges (a
party given contractor or landlord, or a payment that looks like
services to an individual, asks for the role and the W-9).

## After that: updates (check and seamless upgrade)

Decided (the owner, 2026-10-15), after the completeness engine. New versions
fix accounting, tax, and legal rules, so books running an old version may
be wrong; staying current is part of keeping correct books.

- **Check:** the MCP server checks for a newer release when it starts and
  once a day while running (and the viewer and `just session-start` ask
  it). Releases carry notes and a severity: a fix to an accounting, tax, or
  legal rule (a 1099 threshold, a depreciation rule, a refusal that was
  wrong) is marked so, and says which books it may affect. `[updates]
  check = "daily" | "never"` in wanderbooks.toml; the check contacts only
  the release host and sends nothing about the books.
- **Surface:** the update is a completeness item (`books.todo`), in
  `books.digest`, a banner in the viewer, and the session-start hook; the
  MCP instructions tell the agent to raise it with the human at once, with
  the release notes, and to treat a rules fix as urgent.
- **Upgrade:** one step (`wanderbooks update`, or the agent offers it):
  download the release, verify its signature against a key pinned in the
  installed build, then rehearse: open the books read-only with the new
  build and run the full verify (replay, projection, migrations) before
  switching. Swap binaries atomically, keep the previous build for
  rollback, restart the viewer, and have the MCP server exit cleanly so the
  client reconnects to the new one. Never downgrade; never touch the
  journal except through the new build's ordinary migrations.
- **After:** the agent re-runs whatever the fix affects (for example,
  `books.vendor_1099` for the year, or the register) and tells the human
  what changed in their books, if anything.
