# Claims: format

Wanderbooks' claims for bookspec area **format**: what each rule requires,
where the engine enforces it, and which tests check it. The accounting text
lives in bookspec's records (`rules/format/`); here stay the option
wanderbooks picks, its enforcement and tests, and the product rules that are
only wanderbooks' (`WB-format-<n>`).

Rules: B-27, B-34.

---

## B-27 The journal format gate: newer is not corrupt

**retired bookspec:** `BS-format-1`

**Claims:** no (retired 2026-11-08, D-71: wholly wanderbooks'); its one
engine-independent sentence — a reader that does not know a record's format
version refuses it, never misreads it and never calls it corrupt — is now
statement 4 of `BS-ledger-4`.

**Pick:** `older-reader-says-newer`: CHOICE (the owner, 2026-10-16; P0 review,
2026-10-16: a line's own hash is checked before the line is called newer, and
the envelope and its hash are frozen across formats; D-39, 2026-10-20: older
books move to a newer format only with the owner's go-ahead, asked at every
open until given). The retired record's one option names wanderbooks' pick;
the rest of the rule is the product rules below.

**Enforced:** none (the id is retired; the gate is enforced by
`ledger_store::format` under the product rules below).

**Checked:** none at this id (its checks are the product rules').

### Product rules

**WB-format-1 The gate: an older reader stops at the first line it can't read,
says the books are newer, and never calls them corrupt.** Every journal line
has a type, and a version of wanderbooks reads a fixed set of them
(`ledger_store::format::EVENT_TYPES`). A newer version may write a type an
older one doesn't know, or declare a newer journal format with a
`journal.format_set {format, features}` line before the first event that needs
it. An older reader stops at the first line it can't read and says so: it
never skips a line (the books it showed would differ from the record without
saying so), and it never calls newer books corrupt.

What every journal format shares is frozen, so that an older reader can always
follow the chain one line past the first line it can't read. A line is the
envelope `{seq, ts, type, payload, prev}` and nothing beside it; its hash
(what the next line's `prev` holds) is the SHA-256 of its canonical JSON: keys
sorted at every depth, no whitespace. A newer format puts its new fields
inside `payload`, never in the envelope, and never changes how a line is
hashed. A line with a key beside the envelope's is refused as newer, never
read with the key dropped, and its hash covers the key, so the next line still
checks it.

Damage stays corruption. A line's seq and `prev` are checked before the gate,
and its own hash is checked by the next line's `prev`; so when the gate
refuses a line, the reader reads the next one first, taking only its `prev`
(any JSON object with a string `prev`: it may use floats, a string seq or keys
of a format this version doesn't read). If that `prev` doesn't match, the
refused line was altered after it was written (a flipped bit in its type reads
like an unknown type): corrupt. If it matches, the refused line is intact; the
reader says so, and that it can't check the lines after it. If there is no
next line, or it has no `prev` this version can read, nothing checks the
refused line: the reader says it needs a newer wanderbooks or the line is
damaged, and to run verify after updating, and never that it is intact. A
newer format number is newer whatever the rest of its declaration looks like
(a newer format may shape its features differently); a malformed declaration
of a format this version reads is corrupt. A blank line, or one that isn't
UTF-8 JSON, is damage, to the writer and the viewer alike: both read the
journal as bytes split on newlines. A writer never declares a format it
couldn't read back, or lowers the format in force. Opening the books checks
every complete line before it changes anything: a cut-off last line is set
aside only when the lines before it pass, so an older version never alters
newer books.

*Enforced:* `ledger_store::format` (`JournalFormat::admit`, `declared`, `gate`, `settle`, `declaration`), `journal::ENVELOPE_KEYS`, `JournalLine::parse`, `journal::read_journal_and_format`, `Snapshot::read_new`, `Store::open`, `Snapshot::check_disk`, `Store::declare_journal_format`, `StoreError::NewerJournal` (the flag it carries, whether the refused line's own hash was checked, picks the message in `newer_journal_text`).
*Checked:* `ledger-store` `tests/format.rs` (`old_reader_gets_newer_journal`, `a_damaged_line_that_looks_newer_is_corrupt_when_the_next_line_checks_it`, `an_unreadable_last_line_is_reported_as_newer_or_damaged`, `a_newer_line_the_next_line_chains_onto_is_newer_and_intact`, `a_newer_format_frames_its_lines_its_own_way`, `an_unknown_envelope_key_is_newer_and_checked_through`, `a_refused_line_followed_by_a_garbled_line_is_unverified`, `a_line_that_is_not_utf8_is_corrupt_for_the_viewer_and_the_writer`, `an_old_reader_never_repairs_newer_books`, `a_blank_line_is_corrupt_for_the_viewer_and_the_writer`, `damaged_journals_are_still_corrupt`) and the unit tests in `format.rs`; `ledger-gui` `tests/newer_journal.rs`; `ledger-cli` `tests/startup.rs`; the `fuzz_store_replay` target (the writer and the viewer agree on newer versus bad).

**WB-format-2 The journal features: a line an earlier version can't read is
announced before the first line that needs it.** A line an earlier version
can't read is announced by a journal feature
(`ledger_store::format::KNOWN_FEATURES`): a new event type, or a field or
value on an event type an earlier version knows but would misread. The books
declare it in a `journal.format_set` before the first line that needs it, so
an earlier reader stops at the declaration; this version declares every
feature it knows at once (below), never one at a time on the way. P3's:
`members_cumulative` (B-13: a members close counted from its count's first
close, `"rounding": "cumulative_since"`), `account_renamed` (an
`account.renamed`, B-31), `require_tax_category` (a `settings.set` naming
`policy.require_tax_category`, B-32), `tax_categories_gain_exempt` (an
`account.tax_set` naming `form_4797_gain_loss` or `tax_exempt_income`,
categories an earlier version refuses), `disposal_before_service` (an
`asset.disposed` dated before the asset's in-service date, on or after the day
it was acquired: an earlier version refuses a disposal before its purchase
date), `depreciation_catch_up` (an `asset.depreciation_posted` whose amount
isn't its month's own: months skipped or reversed caught up in it, B-15; an
earlier version reads it as the month's own, shows the months it caught up as
not yet run, and running them there posts them twice), `party_creditors`
(`party.creditor_set`: defect 24, B-35), `payment_instruments`
(`instrument.added`, `entry.instrument_set`: defect 12, B-19),
`vendor_1099_decisions` (`vendor.1099_set` with who decided, a
`not_reportable` decision, its reason or the date it takes effect, and
`vendor.w9_flag_set`: defects 18 and 23, D-40, B-19), `schedule_convention` (a
daily schedule's `schedule.registered` carries `convention: daily`; an older
version would read it as equal months, B-16; and a `settings.set` naming
`books.schedule_convention`, a key an earlier version refuses),
`month_reposting` (a schedule month or a depreciation period posted again
after a reversal un-posted it; an older version would refuse the second
`schedule.posted`, and its projection ignore the second
`asset.depreciation_posted`, B-15), `asset_basis` (an `asset.basis_set`, a new
event type: an asset's basis facts with the value at contribution and the
§704(c) method, B-25) and `projects` (a `project.set`, a new event type: a
class's project facts; an `attachment.linked` to the subject kind `project`,
which an older version would call corrupt; a `decision.recorded` with
`projects`, which an older version would drop silently: B-37). After P3:
`document_line_projects` (an `invoice.created` or `bill.created` line carrying
`class_id`, its project, B-37 item 8; an older version would restore the
document without it, silently). It is a feature of its own, not part of
`projects`: books that declared `projects` before it existed wait for their
owner's next upgrade before a document's line names a project. `documents_v2`
(plan P4b S4, one feature for S4 to S7: D-52) is the next: a `settings.set` of
the `invoicing.*`, `bills.*`, `company.email` or `company.website` keys, which
an earlier version refuses (S4.2); a `party.updated` with an email, a default
terms fact (S4.2), or a contact-block fact — a phone, a contact person, a
billing or shipping address, a billing email (D-55); an `invoice.created` or
`bill.created` carrying a number, terms, a bill-to or seller snapshot, a
customer reference or memo, a vendor reference, a receipt date or a
duplicate's reason, or a line naming its account, quantity or unit price
(S4.3); S5's `invoice.sent` (who sent an invoice, how, to whom and when: a
record that moves no figure); S6's payments with `line`, `discount`,
`discount_line`, `discount_reason`, `fee` or `fee_line`; and S7's
`invoice.written_off`. An older version would restore the document without
them silently: every invoice line would post to the revenue role and every
bill line to general expenses, and every new event type would be refused. P6's
is `attachment_text` (an `attachment.text_extracted`, `attachment.annotated`
or `entry.settles`, a new event type, and an `attachment.withdrawn` carrying
`duplicate_of`; an older version would refuse the types, or withdraw a
duplicate without saying what it duplicates, B-44). P4's is `drafts_v2` (an
`entry.draft_state_set` or `entry.draft_discarded`, both new event types: an
older version would refuse them, or, reading around them, keep a discarded
draft that holds a period open and call a waiting draft one being built,
B-43). P5's is `source_revisions` (a `convention.source_revised`,
`decision.source_revised` or `open_item.source_revised`, new event types: an
older version would refuse them, or show the old source as the record's
authority). P8's is `audits` (an `audit.run`, `receipt.attested`,
`entry.reviewed` or `period.attested`, all new event types, and the settings
`policy.attestation_bar` and `policy.cpa_package_requires_audit`: an older
version would refuse the types and the keys, or drop the claims a handoff
rests on, B-45).

A command asks for the feature it needs (`Store::ensure_feature`) before its
first journaled line, and only when the line it is about to write uses it, so
a write refused for waiting on the upgrade journals nothing, never an entry
without the line that says what it is
(`a_write_waiting_for_the_upgrade_journals_nothing`), and a write that needs
nothing new is never refused. A tool journaling several commands as one asks
before the first of them, never between them (a refusal there would leave the
tool's work half done): `books.set_w9` asks for `vendor_1099_decisions` before
the W-9 when the vendor's W-9 flag follows it. A value in a line of a known
type that only a newer version writes (a 1099 decision or reason kind, a
payment method's kind) is newer, never corrupt.

*Enforced:* `ledger_store::format` (`format::declared`, `require_format`, `require_feature`), `Store::ensure_feature`, `Store::declare_journal_format`, `StoreError::FormatUpgradePending`, `format::setting_feature`, `format::tax_category_feature`.
*Checked:* `ledger-store` `tests/settings.rs` (`a_change_needs_journal_format_2`), `tests/format_upgrade.rs` (`a_write_waiting_for_the_upgrade_journals_nothing`, a newer feature still refused); `tests/p2_compat.rs` (every P3 setting key and tax category needs a feature); the unit tests in `format.rs`.

**WB-format-3 The other way round: the format upgrade, with the owner's
go-ahead and a proven backup.** This version's format is the newest it reads
with every feature it knows (`format::current`). Books it creates declare it
in their second line, so they wait for nothing. Books an older version wrote
declare less (format 1, written before the gate, declares nothing): they *wait
for a format upgrade*, which is a fact of their journal alone
(`JournalFormat::upgrade`). They open and replay as they are, byte for byte;
everything that needs nothing new is written as ever, except that since D-88
and D-89 they take no tool write at all (WB-format-11); a write that would
need the newer format or a feature they haven't declared is refused before
anything is journaled (`StoreError::FormatUpgradePending`, guard
`format_upgrade_pending`), never declared on the way. Every setting key and
tax category an earlier version didn't read needs a feature
(`format::setting_feature`, `format::tax_category_feature`: the P3 data-safety
review, 2026-10-21, found `books.schedule_convention`, both new tax
categories, a disposal before the in-service date and a depreciation catch-up
journaled without one, each leaving books P2 refused or misread before
consent; `tests/p2_compat.rs`). Only the owner's go-ahead moves them, with the
owner passphrase (the server checks it, Argon2id): in the app, which asks at
every open until it is given, or with `wanderbooks books upgrade ID` at a
terminal, whose commands that open the books also ask; never an agent, and no
tool. Before the upgrade the server commits the books to their own backup
repository and proves the copy holds them exactly as they are (journal, stored
files, vault and keys, byte for byte), all under the books' lock; a backup
that can't be made or proven upgrades nothing. The upgrade is then one
`journal.format_set` declaring the whole format, so a book's format moves
exactly once per version; books already in it are left alone. An adoption's
copy is new books of this version and moves to its format as it is made (the
original keeps its own). The upgrade never weakens the gate: books declaring a
feature this version doesn't know are newer, as above.

*Enforced:* `ledger_store::format` (`current`, `JournalFormat::upgrade`), `Store::create`, `Store::upgrade_format`, `Root::adopt`, `ledger_store::backup::prove_archive`, ledger-mcp `Backups::upgrade_format` and `local.rs` (the passphrase), ledger-host `host.rs` (the lease intent, handed to the server) and `commands.rs` (`books upgrade`, asking at open), ledger-gui `owner.rs` (the notice and the dialog).
*Checked:* `ledger-store` `tests/format_upgrade.rs` (new books in this version's format, a change waiting for the upgrade, every old-books fixture upgrading to its figures, a newer feature still refused), the unit tests in `format.rs` and `backup.rs` (`the_archive_proves_to_hold_the_book_as_it_is_keys_and_all`), `tests/root.rs` (adoption); `ledger-mcp` `local.rs` (`format_upgrade_waits_for_consent`, `no_upgrade_without_a_backup`, `no_upgrade_unless_the_backup_just_made_is_proven`); `ledger-host` `lease.rs` (`a_consent_never_shows_its_passphrase`); `ledger-cli` `tests/format_upgrade.rs` (from end to end); `ledger-gui` `tests/format_upgrade.rs`.

**WB-format-4 Each projection schema in a file of its own.** (The lead's
decision, 2026-10-21, for 3.9's "before consent, the way back is always
there".) The books' SQLite projection is derived from the journal, and its
schema moves with the version (`schema::SCHEMA_VERSION`), but an older version
refuses a database of a newer schema (`SchemaTooNew`). So each schema keeps
its own file, `wanderbooks.v{N}.db` (`store::db_file_for`): a version opens or
creates only its own; its own missing, it is built from the journal; the
legacy `wanderbooks.db` (every version through P2, schema 12) is used only
while it already holds this version's schema, so books needing no migration
aren't rebuilt for nothing; a file of any other schema, older or newer, is
never migrated, written, refused or deleted, and stays byte for byte for the
version that wrote it (going back finds it as it left it, or builds its own).
A file named for this version's schema that holds another is damaged, kept
aside and rebuilt, as a corrupt one is. `books.verify` (`database`,
`other_databases`) and `wanderbooks doctor` say which file is in use and which
others are there; none is deleted for being stale.

*Enforced:* `store::db_file_for`, `store::database_in_use` (and `Snapshot::check_disk`, which checks the file it names), `Store::database_file`, `Store::other_databases`, `StoreError::SchemaTooNew`.
*Checked:* `ledger-store` `tests/schema_files.rs` (a P2-shaped books dir, its `wanderbooks.db` used at its schema and left byte for byte past it; an older schema's file left byte for byte and whole while this version's is built and kept up; a newer schema's file left alone, not refused; a damaged file of this version's schema rebuilt; new books' file named for their schema).

---

**WB-format-8 Every write a tool makes is one transaction in the journal.**
(The owner, 2026-11-20, D-88: "Writes have to be TRANSACTIONAL ALWAYS! You
can record a failed transaction.") A tool call that writes journal lines
frames them: `call.begun { tool, token, key, args }` just before its first
line, and `call.closed { tool, token, key, args, outcome, first_seq,
last_seq, answer | answer_sha256 + answer_bytes | error }` when it is over,
`outcome` being `ok` (the answer given is `answer`, kept as text so a repeat
returns it byte for byte, when it is at most 4096 bytes; above that the line
keeps `answer_sha256`, the SHA-256 of the answer, and `answer_bytes`, its
length, so every closing line is bounded), `failed` (`error` says how), or
`unfinished` (written by the next open of books whose last call never
closed). `args` is a keyed digest of the call's arguments (WB-format-9). A
call that wrote no journal line has no frame and no closing line: there is
nothing to frame, and nothing it did can be half done. The lines between a
begin and its close count only when the close says `ok`: that is how a reader
tells a complete call from an incomplete one. Every replay (opening the
books, the viewer, verify, rebuild, the projection a restart catches up) goes
through one rule (`CallGate`): a line outside a frame applies at once (every
line before the books declared `call_transactions`, and the owner's own acts:
the format declaration, a repaired tail); a line inside a frame waits, and is
applied with its closing line when that says `ok`, and never when it says
`failed` or `unfinished` or when no closing line follows. So no state is
built from half a call, and a replay of any prefix that ends inside a call
equals the replay of the prefix before it. A close must name the frame it
ends (its `first_seq` is the begin line's, its `last_seq` the line before
it); a frame that begins inside another, a close with no frame, or a close
that does not match is a corrupt journal. The projection keeps a call's
changes in one SQLite transaction, released only after the `ok` closing line
is on disk. When the books are opened and the journal's last call never
closed, its lines are not applied, it is closed `unfinished` before anything
else is written, and a projection that holds its lines is rebuilt. The
envelope `{seq, ts, type, payload, prev}` is untouched: the feature adds two
line types (`call_transactions`, WB-format-2) and nothing else. Calls take
turns: the server runs one write call at a time, so a call's lines are one
run in the journal. `books.import_bank_csv` is one transaction like every
other write (plan P15 6a).
*Proved:* Lean `Calls.prefix_law`, `Calls.replay_drop_frame`, `Calls.replay_ok_frame`, `Calls.call_preserves_inv` (bookspec law 47, `docs/verification.md` law 47): the lines of a call count only when it closed ok, a replay of a prefix inside a call is the replay before its begin, and a call is a run of legal steps.

**Every reader of the journal applies the frame rule, because ids are reused
after a failed call.** A failed call's entry, account, party and document ids
were never taken (nothing it wrote counted), so the next call hands them out
again; a reader that took the raw lines would pin the dead lines of a failed
call on whatever took the id next (a failed `books.create_bill` for entry 42,
then a manual entry 42, would read as a "Bill"). So a reader that derives a
fact (an entry's origin and events, an asset's trail, a party's events, an
attention item, a package, what adopting books would keep) reads the
committed journal (`committed`: the lines outside any call and those of calls
that closed `ok`, the frame lines left out) and never the raw one; a reader
that shows history (the viewer's journal page, `books.journal_history`)
shows every line and says in words of each line of a call that failed, never
finished or is still running that it did not count (`Standing`, `standings`).
Backups and restores copy the journal's bytes whole and read none of its
facts. The reader's view of a running call: a viewer reads a call's lines only
once its closing line is read; a request on the server that arrives between a
call's tool finishing and its closing line, when the call has written a line,
waits for the close, so no read shows what a call did before the call
committed.
*Enforced:* `ledger_store::calls` (`CallGate`, `gate_all`, `standings`, `committed`, `Standing`, `closed_of`, `begun_of`, `ANSWER_CAP`, `Store::begin_call`, `Store::close_call`), `Store::open`, `Store::verify`, `Store::rebuild`, `Snapshot::refresh`, `Snapshot::committed_journal`, `Snapshot::journal_standings`, `Store::journal_history`, `history_standing`, ledger-package `PackageBooks`, ledger-gui `Books::build`, `adopt`, `format::CALL_TRANSACTIONS`, ledger-mcp `SharedStore::write_turn`, `BooksServer::lock`.
*Checked:* `ledger-store` `tests/calls.rs` (`a_call_is_framed_and_its_answer_kept_by_key`, `a_call_that_wrote_nothing_leaves_no_line_and_is_remembered_for_the_session`, `a_long_answer_is_kept_as_its_hash_and_a_repeat_is_told_in_words_what_was_done`, `the_committed_journal_leaves_out_the_lines_of_a_call_that_failed`, `a_crash_at_each_point_of_a_call_leaves_the_whole_call_or_none_of_it`, `a_replay_of_any_prefix_inside_a_call_is_the_replay_of_the_prefix_before_it`, `a_viewer_applies_a_call_only_when_it_closes`, `a_frame_that_does_not_close_what_it_follows_is_corrupt`, `a_call_cannot_begin_inside_another_or_after_a_failed_close`, `a_call_whose_begin_line_the_projection_refuses_still_closes_its_frame`, `books_the_version_before_call_transactions_wrote_open_and_report_the_same_figures`), `tests/store.rs` (`books_written_under_older_rules_still_open`, `books_an_earlier_version_wrote_open_to_its_figures`), the unit test `call_transactions_is_its_own_feature` in `format.rs`, `tests/format.rs` (`every_event_the_store_replays_passes_the_gate`); `ledger-gui` `tests/view_model.rs` (`a_failed_calls_lines_are_not_facts_when_its_ids_are_used_again`: the viewer's books, the feed attention reads, and the history after a failed call and an id handed out again); `ledger-cli` `tests/call_transactions.rs` (`a_call_is_one_frame_in_the_journal_closed_with_its_answer`, `the_import_is_one_transaction`, `a_call_that_wrote_lines_and_then_failed_leaves_the_books_and_its_key_as_they_were`).

**WB-format-9 Every write call carries a request key, and a repeat of it
writes nothing.** Every tool that can change the books (no `readOnlyHint`;
83 of the 140: the three that only read what is sealed, `books.vendor_1099`,
`books.get_w9` and `books.export_cpa_package`, write nothing and may answer with
a full TIN, so they are reads) takes a required `request_key`, 8 to 128 characters of plain
text, which the agent makes up, new for each action. A write without a usable
key is refused in words before it runs; a preview (a confirmed tool called
without `confirmed: true`) writes nothing and needs none. The key is taken out
of the arguments before the tool sees them. A repeat of a (token, key) whose
call committed, with the same tool and arguments, returns that call's answer
as it was given, writes nothing, and is recorded in the access log as
`repeat`; with other arguments or another tool it is refused in words as a
misuse of the key, and nothing is changed. A key is the token's own: two
tokens with one key are two keys. "The same arguments" is a keyed digest
(`args_mac`): HMAC-SHA256 over the arguments' canonical JSON, unmasked, under
a secret of the books' own (`keys/calls.key`, 32 random bytes, made on first
need, mode 0600, never journaled, logged or answered), so two requests that
differ only in a tax id are two requests, and nothing journaled lets anyone
recover or test a guess of a tax id (a plain hash of nine digits would be
guessable). The key file is part of the books' keys folder and travels with
its backups; books restored without it refuse a repeat of an older key as a
misuse of the key (nothing is changed), never as a repeat. The key table is
rebuilt at every open from the `ok` closing lines alone and holds one small
entry per committed call, no answers: a repeat reads its answer from the
closing line. A call that answered `ok` but wrote no journal line leaves no
line, so its key and answer are kept in memory for the life of the server
only, in a table bounded at 512 calls and 2 MiB of answers, oldest first out
(an answer above 4096 bytes is not kept); a repeat within the session gets
the same answer, and after a restart a repeat simply runs again, which is
harmless because it wrote nothing (`books.back_up_now` and `books.begin_upload`
are such calls: a second backup or a second empty upload slot is no harm).
A call that wrote lines and whose answer was above 4096 bytes is repeated in
words: it says the call was done, with its journal lines, the answer's
length and SHA-256, that the answer was too long to keep, and writes
nothing. A repeat that arrives while its original is still running waits for
it (calls take turns) and gets its answer. An answer is kept as given: no
tool answer holds a full TIN, and the three tools that can are no
transaction; an error text that holds one is kept masked (BS-partner-2: no
journal line holds one).
*Enforced:* ledger-mcp `calls::is_write_call`, `calls::key_problem`, `calls::begin`, `calls::close`, `calls::add_key_to_schema`, `Store::recorded_call`, `Keys`, `Replay`, `args_mac`, `Store::args_mac`, `hmac_sha256_hex`, `ANSWER_CAP`, `SESSION_KEYS_MAX`.
*Checked:* `ledger-cli` `tests/call_transactions.rs` (`a_repeat_of_a_key_returns_the_first_answer_and_writes_nothing`, `the_same_key_for_another_request_is_refused_in_words`, `a_write_without_a_usable_key_is_not_run`, `every_tool_that_can_write_asks_for_a_key_and_no_other_does`, `the_same_key_under_two_tokens_is_two_calls`, `a_repeat_arriving_while_its_original_runs_gets_its_answer_and_changes_nothing`, `a_repeat_of_a_call_whose_answer_was_too_long_is_told_it_was_done`, `a_call_that_wrote_nothing_leaves_no_line_and_is_answered_again_only_within_the_session`, `a_key_reused_for_another_tax_id_is_refused_not_repeated`); `ledger-store` `tests/calls.rs` (`a_call_is_framed_and_its_answer_kept_by_key`, `the_sessions_key_table_is_bounded_in_count_and_in_bytes`, `a_long_answer_is_kept_as_its_hash_and_a_repeat_is_told_in_words_what_was_done`) and the unit tests `hmac_sha256_matches_rfc_4231` and `args_mac_tells_tax_ids_apart_and_holds_none` in `calls.rs`.
*Proved:* Lean `Calls.exactly_once`, `Calls.submit_repeat`, `Calls.replay_submit_repeat` (bookspec law 47, `docs/verification.md` law 47): a repeat of a key whose call committed adds nothing to the journal, changes no replay, and no call commits twice.

**WB-format-10 A failed write is on the record, and builds nothing.** A call
that answers with an error, is stopped by a close, or fails any other way
rolls back: if it had written lines, its closing line says `failed` with how,
its lines are never applied, the projection's changes are rolled back and the
live ledger is rebuilt from the journal without them; if it had written none,
the access log has it (as every call has) and the journal has nothing. A
failed call leaves its key unused: nothing happened under it, so the corrected
call may use it. Nothing a call writes outside the journal is final before
the call commits: the tax-id vault's writes (`books.set_tin`, `books.set_w9`,
`books.purge_tin`) are staged under names no reader opens and made final when
the call's closing line is written, or dropped (and a purge put back) when
the call fails; a crash between the closing line and that settling is
finished at the next start by what the journal says of the call, so a failed
or cut call never replaces or destroys a record (`ledger_vault::settle`,
`pending_tags`). A call whose closing line cannot be written has not
committed; the store takes no further write, and the books server stops, as
it does when an access record cannot be written. A call whose closing line
*is* on disk but whose projection cannot be brought up to it has committed:
the store heals from the journal where it can, and where it cannot says so
(`StoreError::CallCommitted`: the call was completed and counts, do not do
it again), stops taking writes, and is rebuilt at the next open.
A call that is refused for a reason that could have been known up front
(a bad argument, a rule that says no) writes nothing: its refusal is decided
before its first line, so the journal is byte for byte what it was and no
failed call stands for it. A failed call on the record is for a call that
failed part way (a fault, a crash), never for one that could have been
refused first. A tool that has several writes to make checks every one of
them before the first (`Store::check_filing_links`,
`Ledger::check_no_party_reason`, the duplicate receipt ids of
`books.dispose_asset`); a test's books holding a failed call fail the test
unless it injected the fault on purpose (`common::expect_failed_calls`).
*Enforced:* `Store::close_call`, `StoreError::CallCommitted`, ledger-mcp `calls::answered_ok`, `calls::close`, `calls::recover_vault`, `calls::put_tin`, `calls::purge_records`, `calls::settle_vault`, `ledger_vault::stage_tin`, `ledger_vault::stage_w9`, `ledger_vault::stage_purge`, `ledger_vault::settle`, `ledger_vault::pending_tags`.
*Checked:* `ledger-store` `tests/calls.rs` (`a_failed_call_is_on_the_record_and_builds_nothing`, `a_call_that_wrote_nothing_leaves_no_line_and_is_remembered_for_the_session`, `a_closing_line_on_disk_but_sqlite_failing_is_a_call_that_counts`); `ledger-vault` `tests/vault.rs` (`a_staged_record_is_read_by_no_one_until_its_call_commits`, `a_call_that_did_not_commit_leaves_the_old_record_whole`, `a_purge_is_final_only_when_its_call_commits`); `ledger-cli` `tests/hostile_args.rs` (`a_refused_call_leaves_the_journal_byte_for_byte_as_it_was`), `tests/spec_attachments.rs` (`filing_bytes_already_linked_is_refused_before_any_link_is_written`), `tests/spec_parties.rs` (`a_no_party_reason_that_carries_a_tax_id_is_refused_before_the_draft_is_written`), `tests/spec_kept.rs` (`a_template_draft_with_a_tax_id_in_its_reason_is_refused_before_it_is_written`), `tests/spec_assets.rs` (`disposing_with_a_receipt_named_twice_is_refused_before_anything_is_written`), `ledger-cli` `tests/call_transactions.rs` (`a_failed_write_leaves_the_books_and_its_key_as_they_were`, `a_call_that_wrote_lines_and_then_failed_leaves_the_books_and_its_key_as_they_were`, `a_crash_with_the_ok_close_on_disk_but_sqlite_not_committed_leaves_the_call_done_once`, `a_set_tin_that_fails_leaves_the_vault_as_it_was`, `a_crash_between_the_commit_and_the_vault_settling_is_finished_at_the_next_start`).
*Proved:* Lean `Calls.replay_drop_frame`, `Calls.keyTable_failed_absent`, `Calls.keyTable_submit_failed` (bookspec law 47, `docs/verification.md` law 47): a failed call counts for nothing and leaves its key out of the table.

**WB-format-11 Books that have not declared `call_transactions` take no
write.** (The owner, 2026-11-20, D-89.) This replaces WB-format-3's "everything
that needs nothing new is written as ever" for writes: from the release that
carries the feature, books an older version wrote are opened and read as
ever, and every write call is refused, before it runs, with the upgrade remedy
and the words "nothing can be written", until the owner's go-ahead (the format
upgrade, WB-format-3). So there is one way a write can happen, the one that is
proved. The Store's own commands, which the owner's acts and the tests call
directly outside a call, are unchanged; `Store::begin_call` refuses.
*Enforced:* `Store::ensure_takes_calls`, `Store::begin_call`, `StoreError::FormatUpgradePending`, ledger-mcp `calls::begin`.
*Checked:* `ledger-store` `tests/calls.rs` (`books_that_have_not_declared_the_feature_take_no_call_and_still_read`), the unit test `call_transactions_is_its_own_feature` in `format.rs`; `ledger-cli` `tests/format_upgrade.rs` (`books_p2_served_cut_over_whole`, `format_upgrade_waits_for_consent`, `documents_v2_refused_until_consent`).

**WB-format-12 Every set of books has a unique id.** (Plan P15 6a: the hash of
the journal's first line, which carries a currency and a time to the second,
told two sets of books apart badly.) Books made since `call_transactions` are
born with `books_id`, 128 random bits in hex, in their `store.created` line.
Older books get one with the format upgrade: the `journal.format_set` that
declares `call_transactions` carries `books_id` when the books have none. The
id is read from the journal alone (`books_id_of`), so it is the same after a
restart and for the viewer, and a restored backup keeps it. Books that have
had neither yet have no id (`Store::books_id` is None).
*Enforced:* `Store::create`, `Store::declare_journal_format`, `Store::books_id`, `Snapshot::books_id`, `books_id_of`.
*Checked:* `ledger-store` `tests/calls.rs` (`new_books_are_born_with_an_id_and_no_two_share_one`, `books_that_have_not_declared_the_feature_take_no_call_and_still_read`), `tests/format.rs` and `tests/format_upgrade.rs` (the upgrade's declaration holds the id).

## B-34 The QuickBooks Desktop file (IIF): plain ASCII, every entry through as_of, closes left out, classes carried

**bookspec:** `BS-format-2`

**Claims:** yes

**Pick:** `plain-ascii-transliterated-and-listed`, `cr-lf`,
`docnum-is-the-entry-id`, `closes-left-out` and `everything-through-as-of`:
CHOICE (the owner, 2026-10-20, D-36): plain ASCII, what isn't ASCII
transliterated (deunicode 1.6.0, pinned exactly, see WB-format-7) and listed,
rather than the books' own text; CR LF, rather than the host's line ending;
DOCNUM the entry's id, rather than the file's place in it; closing entries
left out, rather than written; every entry since the company began, until the
CPA says otherwise. Whether the file should hold only the fiscal year instead
goes to the CPA before the 2027 package (D-36); for the first year the two are
the same. Also CONVENTION: a class name exactly as the books have it is kept
unless another class's own name is the same ignoring case (the P4 follow-ups'
review, finding 4, 2026-10-22: before, all the classes whose names matched
were renamed).

**Pick (BS-format-3):** `plain-ascii-transliterated-and-listed`, the owner, 2026-10-20, D-36.

**Pick (BS-format-4):** `cr-lf`, the owner, 2026-10-20, D-36.

**Pick (BS-format-5):** `docnum-is-the-entry-id`, the owner, 2026-10-20, D-36.

**Pick (BS-format-6):** `everything-through-as-of`, the owner, 2026-10-20, D-36.

**Pick (BS-format-7):** `journal-sample-columns`, the owner, 2026-10-20, D-36 (CHOICE).
**Pick (BS-format-8):** `closes-left-out`, the owner, 2026-10-20, D-36.

**Pick (BS-format-9):** `no-customer-or-vendor-name`, the owner, 2026-10-20, D-36.

**Pick (BS-format-10):** `refuse-the-entry`, the owner, 2026-10-20, D-36.

**Pick:** `no-customer-or-vendor-name`: CHOICE (open risk). The IIF writes no
NAME on a line to accounts receivable or payable, though Intuit's staff say
such a line requires one: no Intuit source says what an IIF journal import
does with a nameless line, so the pick is to omit the name (writing the
party's name instead would need the customer and vendor lists declared too,
with names unique across them) and to name those entries to the CPA
(`Iif::receivable_payable`, the package README). If QuickBooks refuses such a
line on import, the pick is revisited. The other option is writing the party's
name.

**Pick:** `account-types`: the ACCNTTYPE of an account is its QuickBooks
keyword (Intuit's IIF Import Kit `!ACCNT` header help, not vendored, so
CONVENTION): `BANK` for the cash accounts the caller names, `AR` for the
receivables role account, `AP` for the payables role account, `FIXASSET` for
the cost and accumulated depreciation accounts of a registered asset acquired
by the as-of day, otherwise by wanderbooks type `OCASSET` (asset), `OCLIAB`
(liability), `EQUITY`, `INC` (revenue) and `EXP` (expense)
(`ledger_package::export_iif`). The keywords `OASSET`, `CCARD`, `LTLIAB`,
`COGS`, `EXINC` and `EXEXP` are in `ledger_import::QbAccountType` but nothing
in the books maps to them yet.

**Enforced:** `ledger_import::iif::journal_to_iif` refuses an entry with
lines on more than one A/R or A/P account
(`IifError::SeveralReceivablePayableAccounts`), naming every such entry and
saying QuickBooks Desktop records at most one A/R or A/P account a
transaction and that splitting the entry would change what it says;
`ledger_package::export_iif` and `build_package` and `ledger-mcp`
`books.export` and `books.export_cpa_package` return that refusal instead of
a file. **Checked:** `ledger-import` `tests/spec_import.rs`
(`iif_writes_an_entry_on_one_receivable_or_payable_account`,
`iif_refuses_an_entry_on_two_receivable_or_payable_accounts`,
`iif_refuses_an_entry_on_two_receivable_accounts`); `ledger-package`
`tests/package.rs`
(`the_package_refuses_an_entry_on_two_receivable_or_payable_accounts`);
`ledger-cli` `oracle_mcp.rs`
(`iif_and_the_package_refuse_an_entry_on_two_receivable_or_payable_accounts`);
bookspec example
`format-an-export-of-an-entry-on-two-control-accounts-is-refused`
(BS-format-10, the option picked: an entry an earlier version wrote on a
receivable and a payable account is refused for export, read off
`IifError::SeveralReceivablePayableAccounts`), run by
`every_bookspec_example_holds` and
`the_runner_reports_every_claimed_rule_green`.

**Enforced:** `ledger_import::iif` (`field`, `iif_accounts`, `iif_classes`,
`journal_to_iif`, `Iif::warnings`, `Iif::changes`, `Iif::receivable_payable`),
`ledger_package::export_iif` and `build_package`, `ledger-mcp` `books.export`
and `books.export_cpa_package`. **Checked:** `ledger-import` `iif.rs` unit
tests (every one of the 1,112,064 Unicode scalar values written as printable
ASCII, alone and beside a letter, and kept by a second pass; the
transliteration table pinned); `tests/spec_import.rs`
(`iif_docnum_is_the_entry_id`,
`iif_writes_plain_ascii_and_lists_every_change`, `iif_every_line_ends_cr_lf`,
`iif_distinct_accounts_never_share_a_name`,
`iif_a_subaccount_of_another_type_stands_at_the_top_level`,
`iif_declares_each_parent_before_its_subaccounts`,
`iif_fits_quickbooks_desktop_limits`,
`iif_warnings_group_the_memos_by_what_is_written_differently`,
`iif_warnings_never_hold_a_tin`, `iif_change_warnings_are_one_line_each`,
`iif_keeps_a_tin_readable_to_the_package_scan`,
`iif_writes_each_lines_class_and_declares_the_classes_first`,
`iif_class_names_fit_quickbooks_and_every_change_is_listed`,
`iif_class_names_written_as_the_books_have_them_are_kept`,
`iif_refuses_an_undeclared_or_unfit_class`, the properties
`iif_round_trips_random_books`,
`iif_names_never_clash_and_every_change_is_listed` and
`iif_class_names_never_clash`); `ledger-package` `tests/package.rs`
(`iif_leaves_out_period_closes`,
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
`the_default_period_never_shortens_the_iif_or_the_package`); bookspec
examples `format-cr-lf-on-an-lf-platform` and
`format-platform-line-ending-on-an-lf-platform` (BS-format-4),
`format-docnum-is-the-entry-id` and `format-docnum-is-the-files-place`
(BS-format-5), `format-everything-through-as-of-spans-years` and
`format-one-fiscal-year-holds-its-year` (BS-format-6),
`format-closes-left-out` and `format-closes-written` (BS-format-8) and
`format-no-customer-or-vendor-name` (BS-format-9), run by
`every_bookspec_example_holds` and
`the_runner_reports_every_claimed_rule_green`; the line ending is asked by
the iif_line_ending query (the adapter's answer_iif), which reads the file's own bytes: cr-lf when every line break is CR LF, lf when every one is a bare LF, mixed when it is neither. The engine has no notion of a host, so the adapter does not use an example's host_line_ending setup.

### Product rules

**WB-format-5 The tool surface and the package's words.** `books.export` iif
takes `as_of` (default today), never a range, and DOCNUM lets the CPA trace a
QuickBooks transaction to its entry and its receipts (its supporting
documents, B-20). Each change the file reports is said in the `warnings`
of `books.export` and `books.export_cpa_package` and in the package README
(the viewer's note counts them); no line of them holds a full TIN (B-26). The
package README names the entries with a line on an A/R or A/P account
(`Iif::receivable_payable`), so the CPA can add the name if QuickBooks asks,
and its "written differently" paragraph says class names are held to 31
characters, no two alike, and lists each class name written differently with
the accounts' (`class 2's name "Café rebrand" is written "Cafe rebrand"`).
*Enforced:* ledger-mcp `books.export` and `books.export_cpa_package`, `ledger_package::build_package` (the README), `ledger_import::iif::Iif::warnings`.
*Checked:* `ledger-package` `tests/package.rs` (`the_readme_lists_what_the_iif_writes_differently`, `the_readme_names_the_entries_on_receivables_and_payables`); `ledger-import` `tests/spec_import.rs` (`iif_warnings_group_the_memos_by_what_is_written_differently`, `iif_change_warnings_are_one_line_each`, `iif_warnings_never_hold_a_tin`, `iif_class_names_fit_quickbooks_and_every_change_is_listed`); `ledger-cli` `oracle_mcp.rs` (`iif_without_as_of_is_as_of_today_and_ties_to_the_trial_balance`), `mcp_journeys.rs` (`the_default_period_never_shortens_the_iif_or_the_package`).

**WB-format-6 The rest of the package keeps the books' text.** Every other
file in the package keeps the books' text (the PDFs carry Inter and Noto
fallbacks for Chinese, Japanese, Korean, Arabic, Hebrew, Thai and Devanagari,
Arabic and Hebrew set right to left; no emoji font, so an emoji prints as an
empty box). The members' shares are in the package's members' equity statement
and K-1 worksheet, not in the IIF.
*Enforced:* `ledger_import::fonts` (`INTER`, `FALLBACKS`: Inter and the five Noto fallbacks, bundled once in the binary), `ledger_package::build_package`, `ledger_package::reports::member_capitals`, `ledger_package::k1_tax`.
*Checked:* `ledger-package` `tests/package.rs` (the package's files); `ledger-import` `tests/spec_import.rs` (`pdf_keeps_chinese_japanese_and_korean_names`, `pdf_prints_every_bundled_script_as_written`: each of the five fallbacks is embedded, and Arabic and Hebrew read left to right in the extracted text).

**WB-format-7 The transliteration table is deunicode 1.6.0, pinned exactly.**
"René" is "Rene", "北京" is "Bei Jing", "—" is "--", "“" is "'", "😀" is
"grinning". The table is a dependency pinned to one version, so two writers of
the same text write the same characters; the test writes every one of the
1,112,064 Unicode scalar values, alone and beside a letter, and a second pass
keeps it.
*Enforced:* `ledger-import` `Cargo.toml` (`deunicode = { version = "=1.6.0", default-features = false }`), `ledger_import::iif::field`.
*Checked:* `ledger-import` `iif.rs` unit tests (every Unicode scalar value as printable ASCII; the transliteration table pinned).
