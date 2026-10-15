# Parties and attachments

Design for two features built together (Stephen, 2026-10-15): a directory
of the parties the company deals with, and attachments that link to any
subject. Status: design; built after asset tax basis.

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
  id: PartyId,                 // vendors and customers keep their ids (see Migration)
  name: String,                // legal or trade name, as on documents
  display_name: Option<String>,
  roles: BTreeSet<Role>,       // vendor, contractor, customer, employee, member,
                               // payout_platform, tax_agency, financial_institution, landlord
  website: Option<String>,
  address: Option<String>,     // business address; no personal PII beyond it
  w9: Option<W9Facts>,         // contractor/landlord: the TIN itself is in the vault
  w4: Option<W4Facts>,         // employee: never the SSN
  form_1099: Option<Form1099>, // carried over from Vendor
  member: Option<String>,      // a member's name in wanderbooks.toml, when role member
  notes: Option<String>,
  archived: bool,              // no longer dealt with; history stays
}
W9Facts { received: NaiveDate, entity_type: String, exempt_payee_code: Option<String>,
          fatca_code: Option<String>, tin_last4: String, kept_at: String }
W4Facts { received: NaiveDate, state: String, kept_at: String }
```

Rules (to vendor as B-26 with sources):

- A full TIN or SSN is never in the journal: `tin_last4` is exactly four
  digits, and any nine-digit run in any party field is refused (the engine
  checks, not the agent). Full TINs live only in the vault (below).
  Source: IRS truncated-TIN rules (Treas. Reg. §301.6109-4, to vendor) for
  what may appear; DECISION (Stephen, 2026-10-15) for the vault.
- Roles are a set; each role's facts are only allowed with the role
  (W-9 facts need contractor or landlord; W-4 facts need employee).
- Names are unique ignoring case and surrounding space among
  non-archived parties.
- A 1099 needs a contractor or landlord role; 1099 totals come from the
  entries tagged with the party, as today's vendor 1099 report does.

### Links

- Entries: `entry.party_set {entry_id, party_id|null}` generalizes
  `entry.vendor_set`; an entry has one counterparty.
- Invoices and bills already name a customer or vendor id: those are
  party ids after migration.
- Payroll lines name an employee party id (old runs keep their name
  strings; new runs must name a party with the employee role).

### Journal events

`party.added {id, name, roles}`, `party.updated {id, field, value}` (one
field per event: name, display_name, website, address, notes, archived),
`party.role_added {id, role}`, `party.role_removed {id, role}`,
`party.w9_set {id, facts|null}`, `party.w4_set {id, facts|null}`,
`entry.party_set`. Replayed and projected like the rest; every event gets
viewer journal text (the GUI test enforces it).

### Migration

Old journals replay unchanged. `vendor.added` and `customer.added` keep
their meaning and create parties with the vendor or customer role; the
party id space starts above the highest vendor and customer ids, and a
vendor and a customer with the same name stay two parties until the
human merges them (`books.merge_parties`, journaled, re-pointing links).
`entry.vendor_set` replays as `entry.party_set`. `vendor.1099_set` sets
the party's `form_1099` and the contractor role.

### Tools

`books.add_party`, `books.update_party`, `books.party_roles`,
`books.set_w9`, `books.set_w4`, `books.parties` (list, filter by role),
`books.party` (one: facts, roles, totals this year, documents, links),
`books.merge_parties`, `books.set_entry_party`. `books.add_vendor` and
`books.add_customer` stay as shortcuts.

### Viewer

A Parties page (by role, searchable) and a page per party: facts, roles,
year-to-date paid / owed / invoiced / 1099 amount, every entry, invoice,
bill, payroll run, documents, and history. Members link to their capital
accounts. Global search finds parties. Needs-attention: a contractor
paid without W-9 facts; a 1099 party paid over the threshold.

## The TIN vault

Decision (Stephen, 2026-10-15): wanderbooks holds full TINs, so the 1099
worksheet it hands the CPA is complete, but never in the journal.

- **Where:** `vault/` in the books dir, listed in `.gitignore`. Never in
  the hash-chained journal, the SQLite projection, git, an export other
  than the 1099 worksheet, or the viewer. Nothing outside the vault holds
  more than the last four digits.
- **What:** per party, the full TIN, its type (SSN, EIN, ITIN), and the
  W-9 file itself, each encrypted at rest (XChaCha20-Poly1305, one random
  nonce per record; the file format carries a version and the party id as
  associated data, so a record can't be moved to another party).
- **Key:** a random 256-bit key in the OS keyring (Secret Service on Linux,
  Keychain on macOS), created on first use; never on disk in the books dir.
  A recovery copy is the human's job: `just vault-export` writes the key
  and records as one passphrase-encrypted file for a password manager.
- **Journal:** `party.w9_set` records that a W-9 was received, its date,
  entity type, codes, and last four, plus a hash of the vault record, so
  the books prove a W-9 existed without holding it.
- **Purge:** `books.purge_tin {party, reason}` deletes the vault record
  (after the retention period, or on request) and journals that it was
  purged; the journal's facts stay.
- **Use:** only `books.vendor_1099` with `include_tins: true` (and the CPA
  package, when asked) decrypts them. The MCP server never returns a TIN
  in any other tool, log, or error. Tests check that no other output
  contains a nine-digit run.
- **Books without a vault** (another machine, a restored backup) still
  open and verify; the 1099 worksheet then says which TINs are missing.

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
{id, reason}` exists for mistakes. Viewer: per-account reconciliation
history with its statement; Needs-attention: an account with activity not
reconciled through last month's end.

### Import sources

`books.import_bank_csv` files the CSV (kind `import_source`), links it to
every entry it created, and records the row each came from, so every
imported line traces to its source row.

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

Decided (Stephen, 2026-10-15), built after parties and attachments: one
set of completeness rules in the core (asset: tag, serial, tax basis,
acquisition link, label; entry: receipt, 1099 vendor tag; account:
reconciled through last month; month: depreciation, amortization, close;
party: W-9 before paying a contractor; settings: no problems; and the
rules this design adds), `next_steps` on every tool response naming the
exact call that completes each, `books.todo` as the one punch list behind
the digest, the viewer's Needs attention, and the session-start hook, and
enforcement where a rule is a rule (tax basis at registration). The
standard: do it now, completely.

## After that: updates (check and seamless upgrade)

Decided (Stephen, 2026-10-15), after the completeness engine. New versions
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
