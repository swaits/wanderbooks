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
per set of books, with a read-me in plain words, a folder of manifests, and
the pieces the manifests name: the journal and the access log as numbered
segments, an index of the stored files, the stored files themselves under
their hash, and the vault and the keys under their content hash. Every file is
put once: written under a scratch name in its own folder, synced, and given
its name by a hard link, which is made exactly once or refused; where a disk
makes no links a piece is moved into place when its name is free and a
manifest is created exclusively under its number. Nothing in a destination is
ever deleted or rewritten by the program: the only file it removes there is the
scratch file it made a moment before. So a service that syncs the folder never
sees a file change under it, and an earlier state of the books is always still
there (the journal is the history: an earlier state is a shorter prefix, and
each manifest marks one).
*Enforced:* `ledger_store::backup::write_destination`,
`ledger_store::backup::Destination`, `crates/ledger-store/src/backup/io.rs`.
*Checked:* `ledger-store` `tests/backup.rs`
(`a_backup_is_the_book_and_proves_whole`,
`a_program_that_dies_at_any_write_leaves_what_proved_whole_before`),
`tests/backup_props.rs` (`a_destination_only_grows`).

**WB-backup-2 A backup is whole when its manifest is in place, and a manifest
names only pieces that exist and hash right.** The manifest is written last
and atomically. It names the books' id, the head and sequence number it
reaches, every segment, and the vault and key files, each by where it is, how
long it is and what is in it (SHA-256); each manifest stands alone, so one
that is lost, or replaced by a sync service's conflict copy, takes nothing
else with it. A reader takes the newest manifest whose pieces are all there at
the length it names, and the proof re-hashes every piece: the journal's chain
replays, every stored file re-hashes, every TIN record the books name and the
key that opens them are there, the head is the one recorded. A manifest that is
cut short or changed is no manifest. A run that dies leaves pieces no manifest
names, which are harmless.
*Enforced:* `ledger_store::backup::find`, `ledger_store::backup::verify_found`,
`ledger_store::backup::drill`.
*Checked:* `ledger-store` `tests/backup.rs`
(`a_flipped_or_cut_piece_is_found_and_never_restored`,
`the_text_read_from_a_stored_file_is_a_stored_file_too`).

**WB-backup-3 A restore of any whole manifest yields books whose journal is
byte-identical to a prefix of the source's.** A restore previews first and
restores only the head it previewed, as new books or replacing books (which are
moved whole into the legacy folder, never deleted). When the newest manifest is
damaged the newest whole one is used and the preview says how many were passed
over. The backup's journal is a byte prefix of the live one (a drill checks it).
*Enforced:* `ledger_store::backup::verify_found`, `ledger_store::root::Root::restore`,
`ledger_store::root::Root::restore_preview`.
*Checked:* `ledger-store` `tests/backup_props.rs`
(`any_whole_manifest_restores_a_prefix_of_the_books`),
`tests/backup.rs` (`a_damaged_newest_backup_falls_back_to_the_one_before_and_says_so`);
`ledger-cli` `tests/backups.rs` (`a_restore_is_of_the_backup_previewed_only`,
`restore_never_overwrites_and_replacing_keeps_the_old_books`).

**WB-backup-4 A destination of other books, another history of these, or more
than these books have is never written.** Before writing, the destination's
newest whole manifest must be a point on the live journal: the bytes of the
journal it names are the first bytes of the live one. Otherwise the destination
holds other books or a different history, or a newer one than this computer has,
and that is said, and nothing is written. A folder that took these books'
backups and holds none now is a disk not connected, never made again where the
disk should be unless the person says it is a new disk.
*Enforced:* `ledger_store::backup::write_destination`,
`ledger_store::backup::BackupError`.
*Checked:* `ledger-store` `tests/backup.rs`
(`a_destination_of_other_books_or_another_history_is_never_written`,
`a_disk_that_is_gone_is_never_made_again_where_it_should_be`); `ledger-cli`
`tests/backups.rs` (`a_backup_of_another_history_is_never_written_over`,
`restored_books_never_make_their_backup_folder_again`).

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
*Enforced:* `ledger_store::backup::write_destination`,
`crates/ledger-store/src/backup/write.rs`.
*Checked:* `ledger-store` `tests/backup.rs`
(`two_programs_writing_one_destination_do_not_corrupt_it`,
`a_loser_of_a_manifest_number_goes_on_from_the_winner`,
`a_program_that_dies_at_any_write_leaves_what_proved_whole_before`),
`tests/backup_props.rs` (`random_interleavings_of_two_writers_leave_a_whole_backup`).

**WB-backup-6 A flipped or cut piece is found and never restored.** Every piece
is checked against what its manifest says, in length and in hash, and a sealed
piece against its seal; a piece that fails ends the proof of that manifest, and
nothing is put in the books' place from it.
*Enforced:* `ledger_store::backup::verify_found`.
*Checked:* `ledger-store` `tests/backup_props.rs`
(`any_flipped_or_cut_piece_is_found_never_restored`), `tests/backup.rs`
(`a_flipped_or_cut_piece_is_found_and_never_restored`); `ledger-cli`
`tests/backups.rs` (`restore_refuses_a_damaged_backup_and_writes_nothing`,
`restore_drill_failure_is_flagged`).

**WB-backup-7 A passphrase seals every piece, and a wrong passphrase never
yields books.** The passphrase is offered at set-up and is off by default
(D-92). With one, every piece and every manifest is sealed with
XChaCha20-Poly1305 in chunks of 1 MiB, each bound to what is in it and to
whether it is the last. The key is derived from the passphrase with Argon2id at
the owner passphrase's own cost (64 MiB, 3 passes, 1 lane) and a salt of its
own, which the set's header records, so another computer derives the same key.
File names are made from the key and say no more than counts and sizes. The
header carries a sealed check, so a wrong passphrase is told from a damaged
header. The derived key is kept on this computer only, in the book's own
folder, readable by the person's account alone; never in a backup, never in the
journal, never in a record or an error. Changing the passphrase starts a new set
beside the old one and removing it makes the next backup plain: pieces already
written are never rewritten, so the old ones stay as they were, sealed with the
passphrase they were made with. A forgotten passphrase cannot be recovered, and
the set-up says so.
*Enforced:* `ledger_store::backup::BackupKey`,
`crates/ledger-store/src/backup/seal.rs`.
*Checked:* `ledger-store` `tests/backup.rs`
(`a_sealed_backup_gives_away_counts_and_sizes_only`,
`a_wrong_passphrase_never_yields_books_and_is_told_from_damage`,
`a_changed_passphrase_is_a_new_set_beside_the_old_which_is_untouched`,
`the_key_is_kept_privately_and_only_there`), `tests/backup_props.rs`
(`a_wrong_passphrase_never_yields_books`).

**WB-backup-8 "Save a backup..." is the same backup as one file.** The file is a
zip of the set's folder form holding one manifest and only the pieces it names,
sealed when the backups are, with the extension .wbbackup. A file by the name
asked for is never written over, and is moved into place only when whole.
Restore reads a destination folder, a set's own folder, or this file.
*Enforced:* `ledger_store::backup::save_file`, `ledger_store::backup::FILE_EXTENSION`.
*Checked:* `ledger-store` `tests/backup.rs`
(`a_single_file_holds_the_same_backup_plain_or_sealed`).

**WB-backup-9 A stopped backup leaves a destination that proves whole at its
previous manifest, and a close is bounded.** A backup a close stops (the
books' close, or a call or timer's backup when a close begins) looks for the
stop between pieces and once per chunk of a file read, so it ends promptly and
writes no manifest; the server records `stopped` and no result of the run's
own, and the close's own last backup is stopped only at its bound.
*Enforced:* `ledger_store::backup::Ctx`, `ledger_mcp::backup::Backups`.
*Checked:* `ledger-store` `tests/backup.rs`
(`a_stopped_backup_leaves_what_proved_whole_before`); `ledger-cli`
`tests/book_open_close.rs`
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
`a_clone_of_an_old_git_backup_is_adopted_as_a_plain_folder`).

**WB-backup-11 A format upgrade needs a proven local backup, made under the
store's lock.** The local backup is the same form in the book's own folder
(`local-backup`), written and proven to hold the books exactly as they are
(journal, vault and keys byte for byte, every stored file re-hashed) before
the journal's format moves, all under the store's lock.
*Enforced:* `ledger_store::backup::write_local`, `ledger_store::backup::prove_local`,
`ledger_mcp::backup::Backups`.
*Checked:* `ledger-mcp` `local.rs` (`format_upgrade_waits_for_consent`,
`no_upgrade_unless_the_backup_is_proven`, `no_upgrade_without_a_backup`).
