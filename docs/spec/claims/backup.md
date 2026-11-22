# Claims: backups

Wanderbooks' own product rules for backups: plain folders, written once,
optionally sealed with a passphrase (plan P15 section 9, D-91 and D-92,
`plan/p15-app-and-relay.md`). No authority outside this product states them.
They are `WB-backup-<n>`. They replace the rules for backups made with git
(D-30, D-31), which are gone.

Rules: none from bookspec's list; these are product rules only. What a backup
holds (the journal, the stored files, the vault, the keys, the access log;
never the database, which a restore rebuilds) and when it runs are unchanged
(plan 3.7).

---

### Product rules

**WB-backup-1 A backup is plain files, each written once, so a destination
only grows.** A destination is a folder the person picks. It holds one folder
per set of books, named with a fixed prefix and the books' id so that it is a
legal folder name on every system whatever the id is, with a read-me in plain
words, a folder of manifests, and the pieces the manifests name: the journal
and the access log as numbered segments, an index of the stored files, the
stored files themselves under their hash, and the vault and the keys under
their content hash. Every file is put once: written under a scratch name in
its own folder, synced, and given its name by a hard link, which is made
exactly once or refused (WB-backup-18 says what is done on a disk that makes
none). Nothing in a destination is
ever deleted or rewritten by the program: the only file it removes there is the
scratch file it made a moment before. So a service that syncs the folder never
sees a file change under it, and an earlier state of the books is always still
there (the journal is the history: an earlier state is a shorter prefix, and
each manifest marks one).
*Enforced:* `ledger_store::backup::write::write_destination`,
`ledger_store::backup::write::Destination`, `crates/ledger-store/src/backup/io.rs`.
*Checked:* `ledger-store` `tests/backup.rs`
(`a_backup_is_the_book_and_proves_whole`,
`a_program_that_dies_at_any_write_leaves_what_proved_whole_before`),
`tests/backup_props.rs` (`a_destination_only_grows`).

**WB-backup-2 A backup is whole when its manifest is in place, and a manifest
names only pieces that exist and hash right.** The manifest is written last
and atomically. It names the books' id, the head and sequence number it
reaches, every segment, and the vault and key files, each by where it is, how
long it is and what is in it (SHA-256). A manifest names the manifest before
it and only the segments added since, until a chain of 64 is made, when one
names everything again (WB-backup-17); read, every manifest names everything,
and one that is lost, or replaced by a sync service's conflict copy, makes
unreadable only those chained on it, never an earlier one. A reader looks at
the manifests newest first and takes the first whose pieces are all there at the
length it names and that proves whole (it reads no more than that), and the
proof re-hashes every piece: the journal's chain replays to the head the manifest
records before the books are opened, the books open, every stored file re-hashes,
every TIN record the books name and the key that opens them are there. A
manifest that is cut short or changed is no manifest. A run that dies leaves
pieces no manifest names, which are harmless.
*Enforced:* `ledger_store::backup::read::find`, `ledger_store::backup::read::verify_found`,
`ledger_store::backup::read::Set`, `ledger_store::backup::read::prove_books`.
*Checked:* `ledger-store` `tests/backup.rs`
(`a_flipped_or_cut_piece_is_found_and_never_restored`,
`the_text_read_from_a_stored_file_is_a_stored_file_too`), `tests/backup_repair.rs`
(`only_the_newest_backup_is_tried_in_the_good_case`).

**WB-backup-3 A restore of any whole manifest yields books whose journal is
byte-identical to a prefix of the source's.** A restore previews first and
restores only the head it previewed, as new books or replacing books (which are
moved whole into the legacy folder, never deleted). When the newest manifest is
damaged the newest whole one is used, and the preview says so before anything
else (WB-backup-15). The backup's journal is a byte prefix of the live one (a
drill checks it).
*Enforced:* `ledger_store::backup::read::verify_found`, `ledger_store::root::Root::restore`,
`ledger_store::root::Root::restore_preview`.
*Checked:* `ledger-store` `tests/backup_props.rs`
(`any_whole_manifest_restores_a_prefix_of_the_books`),
`tests/backup.rs` (`a_damaged_newest_backup_falls_back_to_the_one_before_and_says_so`);
`ledger-cli` `tests/backups_restore.rs` (`a_restore_is_of_the_backup_previewed_only`,
`restore_never_overwrites_and_replacing_keeps_the_old_books`).

**WB-backup-4 A destination of other books, another history of these, or more
than these books have is never written, and the two are told apart.** Before
writing, the destination's newest whole manifest must be a point on the live
journal: the bytes of the journal it names are the first bytes of the live one.
When the destination's journal is the longer, the live journal is checked to be
the beginning of it: if it is, the books are behind the folder (as after a
restore from an older backup or a backup file) and that is said, in words that
say what to do; if it is not, the books are other books or have diverged, and
that is said. Either way nothing is written. A person who wants to carry on
with the books they have can start a new set of backups beside the old one, in
the same folder, at their explicit request; the old set is never touched, and
later backups go to the new one. A folder that took these books' backups and
holds none now is a disk not connected, never made again where the disk should
be unless the person says it is a new disk.
*Enforced:* `ledger_store::backup::write::write_destination_with`,
`ledger_store::backup::write::Options`, `ledger_store::backup::BackupError`,
`ledger_mcp::backup::Backups`.
*Checked:* `ledger-store` `tests/backup.rs`
(`a_destination_of_other_books_or_another_history_is_never_written`,
`a_disk_that_is_gone_is_never_made_again_where_it_should_be`),
`tests/backup_repair.rs` (`books_behind_the_destination_are_ahead_and_other_books_are_diverged`,
`a_new_set_begins_beside_the_old_at_the_persons_request_and_the_old_is_never_touched`);
`ledger-cli` `tests/backups.rs` (`a_backup_of_another_history_is_never_written_over`),
`tests/backups_restore.rs` (`restored_books_never_make_their_backup_folder_again`),
`tests/backups_trouble.rs`
(`books_restored_from_an_older_backup_are_ahead_and_a_new_set_begins_at_their_request`).

**WB-backup-5 Two programs, or two computers, writing one destination cannot
corrupt it, and a program that dies at any write leaves what proved whole
before.** Pieces are named by what is in them, so two writers of one piece write
the same bytes; the only decision is which writer takes the next manifest
number, and the disk decides it (exclusive create). The loser looks again and
goes on from what the winner wrote, or finds nothing left to write. A program
that dies at any write point (a scratch file, a piece, the manifest) leaves a
destination whose newest whole manifest is the one before, or the new one, and
a restore from it is a prefix of the books. No lock a sync service ignores is
relied on.
*Enforced:* `ledger_store::backup::write::write_destination`,
`crates/ledger-store/src/backup/write.rs`.
*Checked:* `ledger-store` `tests/backup.rs`
(`two_programs_writing_one_destination_do_not_corrupt_it`,
`a_loser_of_a_manifest_number_goes_on_from_the_winner`,
`a_program_that_dies_at_any_write_leaves_what_proved_whole_before`),
`tests/backup_props.rs` (`random_interleavings_of_two_writers_leave_a_whole_backup`).

**WB-backup-6 A flipped or cut piece is found and never restored.** Every piece
is checked against what its manifest says, in length and in hash, and a sealed
piece against its seal; a piece that fails ends the proof of that manifest, and
nothing is put in the books' place from it. A piece is accepted under any
generation of it (WB-backup-13) if its contents hash right.
*Enforced:* `ledger_store::backup::read::verify_found`.
*Checked:* `ledger-store` `tests/backup_props.rs`
(`any_flipped_or_cut_piece_is_found_never_restored`), `tests/backup.rs`
(`a_flipped_or_cut_piece_is_found_and_never_restored`); `ledger-cli`
`tests/backups_restore.rs` (`restore_refuses_a_damaged_backup_and_writes_nothing`),
`tests/backups.rs` (`restore_drill_failure_is_flagged`).

**WB-backup-7 A passphrase seals every piece, and a wrong passphrase never
yields books.** The passphrase is offered at set-up and is off by default
(D-92). With one, every piece and every manifest is sealed with
XChaCha20-Poly1305 in chunks of 1 MiB, each bound to what is in it and to
whether it is the last. The key is derived from the passphrase with Argon2id at
the owner passphrase's own cost (64 MiB, 3 passes, 1 lane) and a salt of its
own, which the set's header records, so another computer derives the same key.
File names are made from the key and say no more than counts and sizes. The
header carries a sealed check bound to the key settings, so a wrong passphrase
is told from a damaged header; a setting the header asks for beyond four times
this program's own (memory, passes, lanes) is refused as damage before any key is
derived, a header that was changed by accident breaks its own check line and
reads as damage, and one changed on purpose can only make the key come out
different, which opens nothing. A folder of sealed sets costs one key derivation
for each distinct setting, not one for each set. Pieces are kept in folders of
the first two digits of their names, so no folder outgrows a small disk's file
system. The derived key is kept on this computer only, in the book's own
folder, readable by the person's account alone; never in a backup, never in the
journal, never in a record or an error. Changing the passphrase starts a new set
beside the old one and removing it makes the next backup plain: pieces already
written are never rewritten, so the old ones stay as they were, sealed with the
passphrase they were made with. A forgotten passphrase cannot be recovered, and
the set-up says so.
*Enforced:* `ledger_store::backup::seal::BackupKey`,
`crates/ledger-store/src/backup/seal.rs`.
*Checked:* `ledger-store` `tests/backup.rs`
(`a_sealed_backup_gives_away_counts_and_sizes_only`,
`a_wrong_passphrase_never_yields_books_and_is_told_from_damage`,
`a_changed_passphrase_is_a_new_set_beside_the_old_which_is_untouched`,
`the_key_is_kept_privately_and_only_there`), `tests/backup_props.rs`
(`a_wrong_passphrase_never_yields_books`), `tests/backup_disk.rs`
(`a_header_cannot_ask_for_more_than_a_few_times_what_this_program_spends`,
`a_header_that_was_changed_reads_as_damage_and_not_as_a_wrong_passphrase`,
`a_folder_of_sealed_sets_costs_one_key_derivation_per_setting_not_one_per_set`,
`sealed_pieces_are_spread_over_folders_as_the_plain_form_does`).

**WB-backup-8 "Save a backup..." is the same backup as one file.** The file is a
zip of the set's folder form holding one manifest and only the pieces it names,
sealed when the backups are, with the extension .wbbackup. A file by the name
asked for is never written over, and is moved into place only when it has been
read back as a restore reads it and proven whole. Restore reads a destination
folder, a set's own folder, or this file.
*Enforced:* `ledger_store::backup::single::save_file`, `ledger_store::backup::single::FILE_EXTENSION`.
*Checked:* `ledger-store` `tests/backup.rs`
(`a_single_file_holds_the_same_backup_plain_or_sealed`).

**WB-backup-9 A stopped backup leaves a destination that proves whole at its
previous manifest, and a close is bounded.** A backup a close stops (the
books' close, or a call or timer's backup when a close begins) looks for the
stop between pieces and once per chunk of a file read, including in the proof
after the backup, so it ends promptly; the server records `stopped` and no
result of the run's own, and the close's own last backup is stopped only at its
bound. A manifest written before the stop is not marked proven: the next
backup proves it, even one with nothing new to write (WB-backup-12). The proof's
scratch books are removed whatever ends it.
*Enforced:* `ledger_store::backup::Ctx`, `ledger_mcp::backup::Backups`.
*Checked:* `ledger-store` `tests/backup.rs`
(`a_stopped_backup_leaves_what_proved_whole_before`), `tests/backup_repair.rs`
(`a_proof_cut_short_by_a_close_ends_cleanly_and_is_made_again_by_the_next_backup`);
`ledger-cli` `tests/book_open_close.rs`
(`a_close_is_not_answered_while_its_backup_still_runs`,
`a_close_stops_a_running_backup_call_and_says_so`).

**WB-backup-10 Older forms are left alone and said.** Books that still carry
the old setting naming a private repository open and work; the backup status
says in plain words that it is no longer used and to choose a folder. An older
backup history in a book's own folder, and an older backup folder made with git
beside a destination, are neither read, written, moved nor deleted; the new form
goes beside them, and the status says once, until a backup has been made in the
new form, that the older one is no longer written. A person who has an older
backup clones it themselves and adopts the plain folder (docs/backups.md).
*Enforced:* `ledger_mcp::backup::Backups`.
*Checked:* `ledger-cli` `tests/backups.rs`
(`old_settings_and_an_old_backup_history_are_left_alone_and_said`,
`a_clone_of_an_old_git_backup_is_adopted_as_a_plain_folder`: it builds a clone's
working tree itself, so it runs, and says nothing is skipped, with no git on the
machine).

**WB-backup-11 A format upgrade needs a proven local backup, made under the
store's lock.** The local backup is the same form in the book's own folder
(`local-backup`), written and proven to hold the books exactly as they are
(journal, vault and keys byte for byte, every stored file re-hashed) before
the journal's format moves, all under the store's lock.
*Enforced:* `ledger_store::backup::write::write_local`, `ledger_store::backup::read::prove_local`,
`ledger_mcp::backup::Backups`.
*Checked:* `ledger-mcp` `local.rs` (`format_upgrade_waits_for_consent`,
`no_upgrade_unless_the_backup_is_proven`, `no_upgrade_without_a_backup`).

**WB-backup-12 "Recorded good" means the backup was read back and proven, and
this is exactly what that proves.** After a manifest is put and before the
backup is recorded good, the manifest is read back from the destination as a
restore would read it, and proven: every journal and access segment it names is
re-hashed and the hash chain replays to the head it records, the books built
from them open, the vault and the keys and their presence are checked (every TIN
record the books name, and the key that opens them), the index of stored files
is read, every stored file the books name is in it, and of the stored files
itself those this backup added and one sixteenth of the others are re-hashed.
The sixteenth is the files whose hash begins with a digit group equal to the
manifest number modulo 16, so any 16 manifest numbers in a row between them re-read
every stored file once: **every stored file is re-read within 16 backups** (the
weekly drill re-reads all of them), and the cost of a backup's proof is the
journal and access log, which the books need anyway, plus a sixteenth of the
stored files. A backup that is not proven is not recorded good. The proof is
marked, per manifest, by a tiny file written once, so a manifest whose proof
was cut short is proven by the next backup. What "recorded good" does not prove:
that a stored file that is not in the share of the 16 is still whole today (the
rotation and the drill will say, and a damaged one is written again, WB-backup-13);
that the destination is still whole after the backup (the next backup and the
weekly drill say). A backup file saved on request is proven whole in full before
it is named.
*Enforced:* `ledger_store::backup::read::prove_written`,
`ledger_store::backup::read::in_rotation`, `ledger_store::backup::read::ROTATION`,
`ledger_store::backup::write::prove_and_repair`, `ledger_store::backup::form::proof_path`,
`ledger_store::backup::single::save_file`, `ledger_mcp::backup::Backups`.
*Checked:* `ledger-store` `src/backup/tests.rs`
(`a_piece_belongs_to_one_share_and_every_share_is_read_within_sixteen_backups`),
`tests/backup_repair.rs`
(`one_damaged_segment_is_found_by_the_next_backup_and_the_drill_does_not_hide_it`,
`a_damaged_stored_file_is_found_by_the_rotation_and_the_next_backup_repairs_it`,
`a_proof_cut_short_by_a_close_ends_cleanly_and_is_made_again_by_the_next_backup`).

**WB-backup-13 A damaged piece is written again beside the damaged file, and a
damaged destination can be repaired.** A piece already in the destination under
its name is taken as the piece when it is put (its name is its hash), so a
damaged one would be taken again by every later backup; the proof after a
backup finds it, and the backup writes the piece again from the books under the
next generation of its name (a name ending .g1, then .g2), a new manifest names
the good one, and the proof is made again. Nothing is rewritten or deleted: the
damaged file is left where it is, and is listed in the status and in the window
as written again, for the person to delete if they wish. A stored file is found
by its hash and generation, a segment, vault record or key by the manifest that
names it, and an index segment that cannot be read is begun again from the
stored files the books have. A damaged piece that can't be written again from the
books (the books' own file is the damaged one) is an error said in words, and
the backup is not recorded good.
*Enforced:* `ledger_store::backup::write::fresh_path`,
`ledger_store::backup::write::prove_and_repair`, `ledger_store::backup::form::generation_path`,
`ledger_store::backup::read::Set`, `ledger_mcp::backup::TargetStatus`.
*Checked:* `ledger-store` `tests/backup_repair.rs`
(`one_damaged_segment_is_found_by_the_next_backup_and_the_drill_does_not_hide_it`,
`a_damaged_stored_file_is_found_by_the_rotation_and_the_next_backup_repairs_it`,
`a_piece_damaged_as_it_is_written_is_repaired_in_the_same_backup`,
`a_damaged_vault_record_and_an_index_segment_are_written_again_too`), `src/backup/tests.rs`
(`generations_and_set_names_round_trip`).

**WB-backup-14 A drill passes only on the newest manifest.** The weekly drill
builds the newest backup's books and proves them whole, re-hashing every stored
file, and passes only if that manifest proves whole and reaches the head of the
last backup recorded good, and its journal is the beginning of the live one.
An older manifest that proves whole while the newest does not is a failure,
never ok: the status and the window say "the newest backup (number N, from
DATE) is damaged; an older one from DATE is whole", and a restore from the
folder would give that older one. A newest manifest that stops short of the last
backup recorded good is a failure too: a backup that was made is not there.
*Enforced:* `ledger_store::backup::read::drill`, `ledger_store::backup::BackupError`,
`ledger_mcp::backup::Backups`.
*Checked:* `ledger-store` `tests/backup_repair.rs`
(`a_drill_never_passes_on_an_older_backup`,
`a_drill_needs_the_newest_backup_to_reach_the_last_one_recorded_good`);
`ledger-cli` `tests/backups_trouble.rs`
(`a_drill_that_finds_only_an_older_backup_is_a_failure_in_words_and_never_ok`).

**WB-backup-15 A restore that is not of the newest backup says so before the
person confirms.** When the newest backup in the folder does not prove whole and
a restore would use an older one, the preview says so first, before anything
else, in the warning colour in the window: this is not the newest backup, how
many newer backups it passes over, when this one was made and when the newest
was, and that anything written after is not in it.
*Enforced:* `ledger_host::commands::older_backup_words`,
`ledger_store::root::RestorePreview`, `ledger_store::backup::read::Verified`.
*Checked:* `ledger-store` `tests/backup_repair.rs` (`a_drill_never_passes_on_an_older_backup`);
`ledger-cli` `tests/backups_trouble.rs`
(`a_drill_that_finds_only_an_older_backup_is_a_failure_in_words_and_never_ok`).

**WB-backup-16 A key lost from a sealed destination never makes backups
plain.** That a destination is sealed is kept where it survives losing the key
file: in the backup record (what was last chosen about the passphrase, and the
key's fingerprint, which opens nothing) and, where the record has nothing, in
the folder itself (it holds sealed sets and none that is plain). A sealed
destination with no key available stops backing up, writes nothing, and says in
words to enter the passphrase or remove it; plain backups are written only after
the person removes the passphrase, which the record keeps.
*Enforced:* `ledger_mcp::backup::Backups`, `ledger_store::backup::write::holds_only_sealed`,
`ledger_store::backup::BackupError`.
*Checked:* `ledger-cli` `tests/backups_trouble.rs`
(`a_lost_key_file_never_makes_backups_plain`).

**WB-backup-17 A manifest names the one before it, and its growth is bounded
without trusting an unproven chain.** No series is ever written again as one
(a series of many small segments is acceptable). A manifest names its base by
number and by what its body hashes to, and the segments added since; a chain is
at most 64 long, then one names everything. A reader proves a chain link by link:
each base must hash to what its successor names and be one link shorter, so a
changed, swapped, missing or looping base makes the manifest unreadable and the
reader takes the newest that reads; no manifest is believed on a chain it has not
read. For a destination of 3000 backups the manifests were 12.1 MB in all
(naming everything each time would be about 765 MB), the largest 413 KB (one in
64 names everything) and the newest 0.8 KB, and the newest was found and read
whole in 32 ms (an unoptimised build; the proof's own time is the journal's).
*Enforced:* `ledger_store::backup::write::kept_form`, `ledger_store::backup::form::DELTA_CHAIN`,
`ledger_store::backup::form::BaseRef`, `ledger_store::backup::read::Set`.
*Checked:* `ledger-store` `src/backup/tests.rs`
(`the_writers_manifests_chain_to_a_bounded_depth_and_are_proven_link_by_link`,
`a_destination_of_thousands_of_backups_stays_small_and_quick_to_read`).

**WB-backup-18 A disk that makes no hard links takes backups too, and a failed
link is never a reason to replace.** Only an error that means the disk makes no
links (on Unix EPERM, ENOSYS, EOPNOTSUPP, ENOTSUP; on Windows
ERROR_INVALID_FUNCTION, ERROR_NOT_SUPPORTED, ERROR_INVALID_PARAMETER,
ERROR_CALL_NOT_IMPLEMENTED) sends a put down the path for such a disk; any other
error is an error. There the finished scratch file is moved into place by a move
that refuses to replace (renameat2 with RENAME_NOREPLACE on Linux, renamex_np
with RENAME_EXCL on macOS, MoveFileExW without the replace flag on Windows); on a
file system that cannot say no, the last resort is exclusive create under the
real name and a copy, which leaves a short file under the name if the program
dies in the middle: the next proof finds it and the piece is written again
(WB-backup-13); a short manifest is accepted by no reader.
*Enforced:* `crates/ledger-store/src/backup/io.rs`,
`ledger_store::backup::Ctx`.
*Checked:* `ledger-store` `tests/backup_disk.rs`
(`a_disk_that_makes_no_links_takes_whole_backups_and_only_grows`,
`two_writers_racing_on_a_disk_that_makes_no_links_do_not_corrupt_it`,
`a_taken_number_or_name_is_never_replaced_on_a_disk_that_makes_no_links`),
`src/backup/tests.rs` (`only_an_error_that_means_no_links_is_taken_to_mean_it`).

**WB-backup-19 A folder a sync service keeps is said in plain words, and a file
it has not downloaded is never read as damage or absence.** When the backup
folder is inside a known sync service's folder (told by the names in its path:
Dropbox, OneDrive, iCloud Drive, Google Drive, Nextcloud, pCloud, Box), and
otherwise in general words, the set-up and the window say: use this folder from
one computer only, keep it available offline, a second computer needs a folder
of its own. A file that is not on this computer yet reads as "not downloaded
yet", the backup waits, and nothing is written over it: on macOS before 14 by
the stand-in name (.name.icloud), on macOS by the dataless file flag, on Windows
by the offline and recall file attributes (read there it would make Windows fetch
it, which may take long or fail, so it is not read). Where nothing can tell
(Linux, whatever client), a file reads as the file it is, and a missing or short
one is found by the proof.
*Enforced:* `ledger_mcp::backup::sync_service`, `ledger_mcp::backup::sync_words`,
`ledger_store::backup::BackupError`, `crates/ledger-store/src/backup/io.rs`.
*Checked:* `ledger-store` `tests/backup_disk.rs`
(`a_file_that_is_not_downloaded_yet_reads_as_such_and_never_as_damage_or_absence`),
`src/backup/tests.rs`
(`what_a_sync_service_leaves_for_a_file_it_has_not_downloaded_is_known`);
`ledger-cli` `tests/backups_trouble.rs`
(`a_folder_a_sync_service_keeps_is_said_in_plain_words_at_set_up`).

**WB-backup-20 The vault's scratch files are never backed up as records.** A
vault write that is between its steps leaves a record staged under a name ending
.staged- and a tag, or an old one set aside under .purging- and a tag. A backup
cut then holds the vault as the journal says it is: a staged record is not the
books' yet and is left out; a record set aside for a purge that has not
committed is still the books' and is backed up under the name it has when it is
not set aside.
*Enforced:* `crates/ledger-store/src/backup/cut.rs`.
*Checked:* `ledger-store` `tests/backup_repair.rs`
(`a_backup_cut_while_the_vault_is_being_written_restores_a_vault_that_matches_the_journal`).

**WB-backup-21 A set's folder is a legal name on every system.** The folder is
named with a fixed prefix and the books' id (con, nul, aux, prn, com1 to com9 and
lpt1 to lpt9 are not names on Windows, but the prefixed names are), and a later
set of the same books ends in a dot and a number, which no id holds.
*Enforced:* `ledger_store::backup::form::plain_set_name`,
`ledger_store::backup::write::set_name_for`.
*Checked:* `ledger-store` `tests/backup_repair.rs`
(`a_set_folder_is_a_legal_name_whatever_the_books_are_called`).
