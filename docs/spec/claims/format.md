# Claims: format

Wanderbooks' claims for bookspec area **format**: what each rule requires, where the engine enforces it, and which tests check it. Being reduced to the engine's pick and its evidence as P14 unit S-D lands; the accounting text moves to bookspec's `spec/format.md`.

Rules: B-27, B-34.

---

## B-27 The journal format gate: newer is not corrupt

**bookspec:** `BS-format-1`

**Status:** CHOICE (the owner, 2026-10-16; `plan/served.md` 5.1 and P0);
CHOICE (P0 review, 2026-10-16: a line's own hash is checked before the
line is called newer; the envelope and its hash are frozen across
formats); CHOICE (the owner, 2026-10-20, D-39, and `plan/served.md`
3.9: older books move to a newer format only with the owner's go-ahead,
asked at every open until given)

Every journal line has a type, and a version of wanderbooks reads a fixed
set of them (`ledger_store::format::EVENT_TYPES`). A newer version may
write a type an older one doesn't know, or declare a newer journal format
with a `journal.format_set {format, features}` line before the first event
that needs it. An older reader stops at the first line it can't read and
says so: it never skips a line (the books it showed would differ from the
record without saying so), and it never calls newer books corrupt.

What every journal format shares is frozen, so that an older reader can
always follow the chain one line past the first line it can't read. A
line is the envelope `{seq, ts, type, payload, prev}` and nothing beside
it; its hash (what the next line's `prev` holds) is the SHA-256 of its
canonical JSON: keys sorted at every depth, no whitespace. A newer format
puts its new fields inside `payload`, never in the envelope, and never
changes how a line is hashed. A line with a key beside the envelope's is
refused as newer, never read with the key dropped, and its hash covers
the key, so the next line still checks it.

Damage stays corruption. A line's seq and `prev` are checked before the
gate, and its own hash is checked by the next line's `prev`; so when the
gate refuses a line, the reader reads the next one first, taking only its
`prev` (any JSON object with a string `prev`: it may use floats, a string
seq or keys of a format this version doesn't read). If that `prev` doesn't
match, the refused line was altered after it was written (a flipped bit in
its type reads like an unknown type): corrupt. If it matches, the refused
line is intact; the reader says so, and that it can't check the lines
after it. If there is no next line, or it has no `prev` this version can
read, nothing checks the refused line: the reader says it needs a newer
wanderbooks or the line is damaged, and to run verify after updating, and
never that it is intact. A newer format number is newer whatever the rest
of its declaration looks like (a newer format may shape its features
differently); a malformed declaration of a format this version reads is
corrupt. A blank line, or one that isn't UTF-8 JSON, is damage, to the
writer and the viewer alike: both read the journal as bytes split on
newlines. A writer never declares a format it couldn't read back, or
lowers the format in force. Opening the books checks every complete line
before it changes anything: a cut-off last line is set aside only when the
lines before it pass, so an older version never alters newer books.

A line an earlier version can't read is announced by a journal feature
(`ledger_store::format::KNOWN_FEATURES`): a new event type, or a field or
value on an event type an earlier version knows but would misread. The
books declare it in a `journal.format_set` before the first line that
needs it, so an earlier reader stops at the declaration; this version
declares every feature it knows at once (below), never one at a time on
the way. P3's: `members_cumulative` (B-13: a members close counted from
its count's first close, `"rounding": "cumulative_since"`),
`account_renamed` (an `account.renamed`, B-31), `require_tax_category` (a
`settings.set` naming `policy.require_tax_category`, B-32),
`tax_categories_gain_exempt` (an `account.tax_set` naming
`form_4797_gain_loss` or `tax_exempt_income`, categories an earlier version
refuses), `disposal_before_service` (an `asset.disposed` dated before the
asset's in-service date, on or after the day it was acquired: an earlier
version refuses a disposal before its purchase date), `depreciation_catch_up`
(an `asset.depreciation_posted` whose amount isn't its month's own: months
skipped or reversed caught up in it, B-15; an earlier version reads it as the
month's own, shows the months it caught up as not yet run, and running them
there posts them twice),
`party_creditors` (`party.creditor_set`: defect 24, B-35),
`payment_instruments` (`instrument.added`, `entry.instrument_set`:
defect 12, B-19), `vendor_1099_decisions` (`vendor.1099_set` with who
decided, a `not_reportable` decision, its reason or the date it takes
effect, and `vendor.w9_flag_set`: defects 18 and 23, D-40, B-19),
`schedule_convention` (a daily schedule's `schedule.registered` carries
`convention: daily`; an older version would read it as equal months,
B-16; and a `settings.set` naming `books.schedule_convention`, a key an
earlier version refuses), `month_reposting` (a schedule month or a depreciation period
posted again after a reversal un-posted it; an older version would refuse
the second `schedule.posted`, and its projection ignore the second
`asset.depreciation_posted`, B-15), `asset_basis` (an
`asset.basis_set`, a new event type: an asset's basis facts with the
value at contribution and the §704(c) method, B-25) and `projects` (a
`project.set`, a new event type: a class's project facts; an
`attachment.linked` to the subject kind `project`, which an older version
would call corrupt; a `decision.recorded` with `projects`, which an older
version would drop silently: B-37). After P3: `document_line_projects`
(an `invoice.created` or `bill.created` line carrying `class_id`, its
project, B-37 item 8; an older version would restore the document without
it, silently). It is a feature of its own, not part of `projects`: books
that declared `projects` before it existed wait for their owner's next
upgrade before a document's line names a project. `documents_v2` (plan
P4b S4, one feature for S4 to S7: D-52) is the next: a `settings.set` of
the `invoicing.*`, `bills.*`, `company.email` or `company.website` keys,
which an earlier version refuses (S4.2); a `party.updated` with an email,
a default terms fact (S4.2), or a contact-block fact — a phone, a contact
person, a billing or shipping address, a billing email (D-55); an
`invoice.created` or `bill.created`
carrying a number, terms, a bill-to or seller snapshot, a customer
reference or memo, a vendor reference, a receipt date or a duplicate's
reason, or a line naming its account, quantity or unit price (S4.3);
S5's `invoice.sent` (who sent an invoice, how, to whom and when: a record
that moves no figure); S6's payments with `line`, `discount`,
`discount_line`, `discount_reason`, `fee` or `fee_line`; and S7's
`invoice.written_off`. An older version would
restore the document without them silently: every invoice line would post
to the revenue role and every bill line to general expenses, and every new
event type would be refused. P6's is `attachment_text` (an
`attachment.text_extracted`, `attachment.annotated` or `entry.settles`, a
new event type, and an `attachment.withdrawn` carrying `duplicate_of`; an
older version would refuse the types, or withdraw a duplicate without
saying what it duplicates, B-44). P4's is `drafts_v2` (an
`entry.draft_state_set` or `entry.draft_discarded`, both new event types:
an older version would refuse them, or, reading around them, keep a
discarded draft that holds a period open and call a waiting draft one
being built, B-43). P5's is `source_revisions` (a
`convention.source_revised`, `decision.source_revised` or
`open_item.source_revised`, new event types: an older version would
refuse them, or show the old source as the record's authority). A command asks for
being built, B-43). P8's is `audits` (an `audit.run`, `receipt.attested`,
`entry.reviewed` or `period.attested`, all new event types, and the
settings `policy.attestation_bar` and `policy.cpa_package_requires_audit`:
an older version would refuse the types and the keys, or drop the claims a
handoff rests on, B-45). A command asks for
the feature it needs (`Store::ensure_feature`) before its first journaled
line, and only when the line it is about to write uses it, so a write
refused for waiting on the upgrade journals nothing, never an entry
without the line that says what it is
(`a_write_waiting_for_the_upgrade_journals_nothing`), and a write that
needs nothing new is never refused. A tool journaling
several commands as one asks before the first of them, never between
them (a refusal there would leave the tool's work half done):
`books.set_w9` asks for `vendor_1099_decisions` before the W-9 when the
vendor's W-9 flag follows it. A value in a line of a known type that only
a newer version writes (a 1099 decision or reason kind, a payment
method's kind) is newer, never corrupt.

**The other way round: the format upgrade.** This version's format is
the newest it reads with every feature it knows (`format::current`). Books
it creates declare it in their second line, so they wait for nothing.
Books an older version wrote declare less (format 1, written before the
gate, declares nothing): they *wait for a format upgrade*, which is a fact
of their journal alone (`JournalFormat::upgrade`). They open and replay as
they are, byte for byte; everything that needs nothing new is written as
ever; a write that would need the newer format or a feature they haven't
declared is refused before anything is journaled
(`StoreError::FormatUpgradePending`, guard `format_upgrade_pending`), never
declared on the way. Every setting key and tax category an earlier version
didn't read needs a feature (`format::setting_feature`,
`format::tax_category_feature`: the P3 data-safety review, 2026-10-21,
found `books.schedule_convention`, both new tax categories, a disposal
before the in-service date and a depreciation catch-up journaled without
one, each leaving books P2 refused or misread before consent;
`tests/p2_compat.rs`). Only the owner's go-ahead moves them, with the owner
passphrase (the server checks it, Argon2id): in the app, which asks at
every open until it is given, or with `wanderbooks books upgrade ID` at a
terminal, whose commands that open the books also ask; never an agent, and
no tool. Before the upgrade the server commits the books to their own
backup repository and proves the copy holds them exactly as they are
(journal, stored files, vault and keys, byte for byte), all under the
books' lock; a backup that can't be made or proven upgrades nothing. The
upgrade is then one `journal.format_set` declaring the whole format, so a
book's format moves exactly once per version; books already in it are left
alone. An adoption's copy is new books of this version and moves to its
format as it is made (the original keeps its own). The upgrade never
weakens the gate: books declaring a feature this version doesn't know are
newer, as above.

**Each projection schema in a file of its own** (the lead's decision,
2026-10-21, for 3.9's "before consent, the way back is always there"). The
books' SQLite projection is derived from the journal, and its schema moves
with the version (`schema::SCHEMA_VERSION`), but an older version refuses a
database of a newer schema (`SchemaTooNew`). So each schema keeps its own
file, `wanderbooks.v{N}.db` (`store::db_file_for`): a version opens or
creates only its own; its own missing, it is built from the journal; the
legacy `wanderbooks.db` (every version through P2, schema 12) is used only
while it already holds this version's schema, so books needing no
migration aren't rebuilt for nothing; a file of any other schema, older or
newer, is never migrated, written, refused or deleted, and stays byte for
byte for the version that wrote it (going back finds it as it left it, or
builds its own). A file named for this version's schema that holds another
is damaged, kept aside and rebuilt, as a corrupt one is. `books.verify`
(`database`, `other_databases`) and `wanderbooks doctor` say which file is
in use and which others are there; none is deleted for being stale.

**Enforced:** `ledger_store::format` (`JournalFormat::admit`, `declared`,
`gate`, `settle`, `declaration`, `current`, `JournalFormat::upgrade`,
`require_format`, `require_feature`), `journal::ENVELOPE_KEYS`,
`JournalLine::parse`, `journal::read_journal_and_format`,
`Snapshot::read_new`, `Store::open`, `Store::create`, `store::db_file_for`,
`store::database_in_use` (and `Snapshot::check_disk`, which checks the file
it names), `Store::database_file`, `Store::other_databases`,
`Store::declare_journal_format`, `Store::upgrade_format`,
`Store::ensure_feature`, `Root::adopt`, `StoreError::NewerJournal` (its
`verified` flag picks the message), `StoreError::FormatUpgradePending`,
`ledger_store::backup::prove_archive`, ledger-mcp `Backups::upgrade_format`
and `local.rs` (the passphrase), ledger-host `host.rs` (the lease intent,
handed to the server) and `commands.rs` (`books upgrade`, asking at open),
ledger-gui `owner.rs` (the notice and the dialog).
**Checked:** `ledger-store` `tests/format.rs` (`old_reader_gets_newer_journal`,
`a_damaged_line_that_looks_newer_is_corrupt_when_the_next_line_checks_it`,
`an_unreadable_last_line_is_reported_as_newer_or_damaged`,
`a_newer_line_the_next_line_chains_onto_is_newer_and_intact`,
`a_newer_format_frames_its_lines_its_own_way`,
`an_unknown_envelope_key_is_newer_and_checked_through`,
`a_refused_line_followed_by_a_garbled_line_is_unverified`,
`a_line_that_is_not_utf8_is_corrupt_for_the_viewer_and_the_writer`,
`an_old_reader_never_repairs_newer_books`,
`a_blank_line_is_corrupt_for_the_viewer_and_the_writer`,
`damaged_journals_are_still_corrupt`) and the unit tests in `format.rs`;
`ledger-gui` `tests/newer_journal.rs`; `ledger-cli` `tests/startup.rs`;
the `fuzz_store_replay` target (the writer and the viewer agree on newer
versus bad). The upgrade: `ledger-store` `tests/format_upgrade.rs` (new
books in this version's format, a change waiting for the upgrade, every
old-books fixture upgrading to its figures, a newer feature still
refused), the unit tests in `format.rs` and `backup.rs`
(`the_archive_proves_to_hold_the_book_as_it_is_keys_and_all`),
`tests/settings.rs` (`a_change_needs_journal_format_2`), `tests/root.rs`
(adoption), `tests/schema_files.rs` (a P2-shaped books dir, its
`wanderbooks.db` used at its schema and left byte for byte past it; an
older schema's file left byte for byte and whole while this version's is
built and kept up; a newer schema's file left alone, not refused; a damaged
file of this version's schema rebuilt; new books' file named for their
schema); `ledger-mcp` `local.rs` (`format_upgrade_waits_for_consent`,
`no_upgrade_without_a_backup`,
`no_upgrade_unless_the_backup_just_made_is_proven`); `ledger-host`
`lease.rs` (`a_consent_never_shows_its_passphrase`); `ledger-cli`
`tests/format_upgrade.rs`
(from end to end); `ledger-gui` `tests/format_upgrade.rs`.

## B-34 The QuickBooks Desktop file (IIF): plain ASCII, every entry through as_of, closes left out, classes carried

**bookspec:** `BS-format-2`

**Status:** AUTHORITY (Intuit, `sources/intuit-iif.md`: IIF files are
ASCII text, tab-separated, with no escape; an account is known by its
NAME, a subaccount's NAME its path joined by colons; lists come before
what uses them; DOCNUM is "the transaction's number"; the kit's 2007+
example file ends every line CR LF; QuickBooks Desktop holds an account
name of 31 characters, an account number of 7 and a memo of 4,095; "The
parent and subaccount must be the same account type"; a journal entry
to accounts receivable or payable needs a customer or vendor, and takes
one such account a transaction; a class list's keyword is CLASS, "Only
the NAME column is required", a subclass's NAME its parents' first,
joined by colons, and the kit declares its classes before the
transactions that name them, line by line in CLASS; Intuit's qbXML
Onscreen Reference, `ClassAdd`: a class's Name at most 31 characters in
QuickBooks Desktop, as an account's); CONVENTION (a
QuickBooks integrator and an IIF importer, same file: a longer name is
cut on a chart import but not in a journal's lines; a double quote is a
text delimiter; Conductor's QuickBooks Desktop API: a class's name
case-insensitive, so no two alike ignoring case); CHOICE (owner, 2026-10-20, plan D-36: plain ASCII, what isn't
ASCII transliterated and listed; CR LF; DOCNUM the entry's id; closing
entries left out; every entry since the company began until the CPA
says otherwise); Unicode Standard Annex #14 (the mandatory line breaks)

1. **One transaction per posted entry dated on or before as_of**, from
   the first, in the order posted, **except period-closing entries**:
   QuickBooks keeps no closing entries (it works out retained earnings
   itself; its "closing date" only locks a period), so an imported close
   would zero its period's profit. The file's balances are the trial
   balance as of as_of without the closes, and its income and expenses
   each period's profit and loss; the members' shares are in the
   package's members' equity statement and K-1 worksheet.
   `books.export` iif takes as_of (default today), never a range.
   Whether the file should hold only the fiscal year instead goes to the
   CPA before the 2027 package (D-36); for the first year the two are
   the same.
2. **DOCNUM is the entry's id** in the books, on the TRNS and every SPL,
   so the CPA can trace a QuickBooks transaction to its entry and its
   receipts.
3. **The file is ASCII and every line ends CR LF.** In a field (a NAME,
   an ACCNUM, a MEMO) a tab or a mandatory line break (LF, VT, FF, CR,
   NEL, U+2028, U+2029) is a space; any other control character is '?';
   printable ASCII is itself, but a double quote, which is a single one
   (QuickBooks reads a double quote as a text delimiter); any other
   character is its transliteration by deunicode 1.6.0, pinned exactly
   ("René" "Rene", "北京" "Bei Jing", "—" "--", "“" "'", "😀" "grinning";
   a transliterated word's trailing space dropped at the end or before a
   space), or '?' when it has none. Every other file in the package keeps
   the books' text (the PDFs carry Inter and Noto fallbacks for Chinese,
   Japanese, Korean, Arabic, Hebrew, Thai and Devanagari, Arabic and
   Hebrew set right to left; no emoji font, so an emoji prints as an
   empty box).
4. **An account's own name**, so written, has each ':' replaced by '-'
   (a fullwidth colon too), is trimmed ('?' if nothing is left) and cut
   to 31 characters (QuickBooks Desktop's limit; cut by QuickBooks, the
   ACCNT line and the transactions' would no longer match); its NAME is
   its parents' NAMEs and its own, joined by ':'. A subaccount whose
   QuickBooks type differs from its parent's (a bank account under a
   header that is no bank, an asset's cost under a "Fixed assets" header)
   is written at the top level, its own subaccounts with it: QuickBooks
   gives a subaccount its parent's type, and the type decides where its
   balance shows. Each parent is declared before its subaccounts, the
   accounts otherwise in the order of their codes. Accounts whose NAMEs
   would match ignoring case, which QuickBooks would merge, get
   " (code)" after their own names (the name cut to make room, the code
   written like a name, "no number" for a blank one), level by level from
   the top and again until no two match; a match left after that (two
   codes written alike, ignoring case, or alike in their first 30
   characters) is refused.
5. **ACCNUM is the account's code**, written as a field and trimmed, or
   left blank (ACCNUM is optional; the NAME identifies the account) when
   it is longer than 7 characters (QuickBooks Desktop's limit) or another
   account's would be written the same, ignoring case. **A MEMO** is cut
   to 4,095 characters (QuickBooks Desktop's limit).
6. **Nothing changes silently.** Each account name, number or place and
   each memo the file writes differently from the books (`Iif::changes`)
   is said, both texts quoted and escaped, one line each, in the
   `warnings` of `books.export` and `books.export_cpa_package` and in the
   package README (the viewer's note counts them): each account's name,
   number and place a line; the memos grouped by what is written
   differently, each run of changed characters once with every entry it
   is in (`"—" is written "--" in the memos of entries 3, 5, 9`), then
   the memos cut. No line holds a full TIN (B-26): a run of nine digits,
   in any script's digits and separators, is masked, and so is every
   numeral of a text that holds one once written as a field (digits a
   tab apart, letters deunicode writes as digits); a memo line whose
   digits are masked says so.
7. **No customer or vendor NAME.** Intuit's IIF help makes NAME
   optional and states no need of it in a journal; QuickBooks Desktop
   asks for a customer or vendor on a journal entry to accounts
   receivable or payable, and takes one A/R or A/P account a transaction
   (Intuit staff and help, `sources/intuit-iif.md`). Writing names would
   need the customer and vendor lists declared as well, with names unique
   across them. So the file names none, and the package README names the
   entries with a line on an A/R or A/P account (`Iif::receivable_payable`),
   so the CPA can add the name if QuickBooks asks.
8. **Each line's class (project, B-37) is its CLASS**, and every class in
   the books is declared in a `!CLASS` list (NAME) after the accounts and
   before the transactions, as the kit declares its lists; books with no
   class write no list. A class's NAME is written as an account's own
   name is (rules 3 and 4: plain ASCII, a ':' a '-' since a colon starts
   a subclass, trimmed, '?' if nothing is left, cut to 31 characters);
   classes whose NAMEs would match ignoring case get " (id)" after their
   names (the name cut to make room), round after round until none does,
   as accounts get " (code)", except that among names that match, those
   already written differently from the books (made safe, or tagged)
   yield first, another " (id)" each time need be (CONVENTION: the CPA sees
   the books' own names wherever they can stand): a class's name
   written exactly as the books have it is kept unless another class's
   own name is the same ignoring case (classes 1 "Café", 2 "Cafe" and 3
   "Cafe (1)": "Cafe (1) (1)", the other two kept; review of the P4 follow-ups, finding 4,
   2026-10-22: before, all three were renamed; and a fourth "Cafe (1) (1)"
   makes class 1 "Cafe (1) (1) (1)", where a cap of two tags once renamed
   class 4 instead). A tag that would change nothing (a name cut to 31
   characters, ending in its own " (id)") isn't made: the name it meets
   is tagged instead. Each class name written differently is
   listed with rule 6's (`class 2's name "Café rebrand" is written "Cafe
   rebrand"`), and the package README's "written differently" paragraph
   says class names are held to 31 characters, no two alike. A line
   whose class isn't declared is refused, never written classless. Before
   2026-10-22 the file left CLASS empty on every line and the README
   didn't say so (the P3 cutover review's open point 1).

**Enforced:** `ledger_import::iif` (`field`, `iif_accounts`,
`iif_classes`, `journal_to_iif`, `Iif::warnings`), `ledger_package::export_iif` and
`build_package`, `ledger-mcp` `books.export` and
`books.export_cpa_package`.
**Checked:** `ledger-import` `iif.rs` unit tests (every one of the
1,112,064 Unicode scalar values written as printable ASCII, alone and
beside a letter, and kept by a second pass; the transliteration table
pinned); `tests/spec_import.rs` (`iif_docnum_is_the_entry_id`,
`iif_writes_plain_ascii_and_lists_every_change`,
`iif_every_line_ends_cr_lf`, `iif_distinct_accounts_never_share_a_name`,
`iif_a_subaccount_of_another_type_stands_at_the_top_level`,
`iif_declares_each_parent_before_its_subaccounts`,
`iif_fits_quickbooks_desktop_limits`,
`iif_warnings_group_the_memos_by_what_is_written_differently`,
`iif_warnings_never_hold_a_tin`,
`iif_change_warnings_are_one_line_each`,
`iif_keeps_a_tin_readable_to_the_package_scan`,
`iif_writes_each_lines_class_and_declares_the_classes_first`,
`iif_class_names_fit_quickbooks_and_every_change_is_listed`,
`iif_class_names_written_as_the_books_have_them_are_kept`,
`iif_refuses_an_undeclared_or_unfit_class`, the properties
`iif_round_trips_random_books`,
`iif_names_never_clash_and_every_change_is_listed` and
`iif_class_names_never_clash`); `ledger-package`
`tests/package.rs` (`iif_leaves_out_period_closes`,
`the_readme_lists_what_the_iif_writes_differently`,
`the_readme_names_the_entries_on_receivables_and_payables`,
`the_package_iif_carries_each_lines_project`,
`the_bookspec_iif_example_matches_the_engine`, and B-33's); the bookspec
example `bookspec/examples/iif-quickbooks-journal-file.toml` (formats
appendix: the file line by line and its changes, derived by hand);
`ledger-cli` `oracle_mcp.rs`
(`iif_declares_accounts_by_role_and_balances_every_transaction`,
`iif_without_as_of_is_as_of_today_and_ties_to_the_trial_balance`),
`mcp_journeys.rs` (`mcp_import_export`,
`the_default_period_never_shortens_the_iif_or_the_package`).

