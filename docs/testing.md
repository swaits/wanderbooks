# Testing strategy

How wanderbooks knows its books are right. Strongest oracle first: a
test's worth is where its idea of "correct" comes from, not what it runs.
Formal proofs are in `docs/verification.md`; the authorities every rule
comes from are in `docs/spec/`. Every crate is covered: `ledger-core`, the
store, import, config, and MCP in full; the GUI where it computes and
writes numbers (its view model and formatting) and in its read-only
promise.

## Layers

| Layer | Oracle (rank) | Where | Command | Runtime | Gates |
|---|---|---|---|---|---|
| Spec-derived examples | written specs with sources (1) | `tests/spec_*.rs`, `invariants.rs`, `statements.rs`, `assets.rs`, ...; the store's `tests/spec_store.rs` (written blind, from its specs only) | `just check` | seconds | every change |
| Regressions | the bug, named (1) | beside the code they cover, `*_regression`/named for the defect | `just check` | seconds | every change |
| Compatibility contracts | journals written under older rules must open, and replay to the figures their version computed (1) | `ledger-store/tests/store.rs` (`books_written_under_older_rules_still_open`; `books_an_earlier_version_wrote_open_to_its_figures` over `tests/fixtures/old-books`: books main 188c90ea wrote, with main's own figures in `expected.json`, in the writer and the viewer; `every_figure_over_old_books_is_computed_or_refused`); `ledger-mcp/tests/hostile_args.rs` `old_books_answer_every_tool`; `ledger-gui/tests/old_books.rs` | `just check` | ~30s | every change |
| Properties | laws over all inputs (2) | `properties.rs`, `spec_*.rs`, `kernel_bridge.rs` | `just check` | seconds | every change |
| Metamorphic | scaling, reversal, ordering invariance (2) | `spec_reports.rs` | `just check` | seconds | every change |
| Refusal law (store) | a refused command never reaches the journal, an accepted one always does; the live ledger equals its journal's replay in every part, and the books reopen to it (2, 3) | `store.rs` `refused_commands_never_reach_the_journal` | `just check` (`STORE_CASES=5000` deeper) | ~20s | every change |
| Fault injection (store) | a full disk or a cut-off append leaves whole, openable books (1) | `ledger-store/tests/faults.rs`, `store.rs` (`power_loss_at_any_byte_of_the_last_append_is_recovered`) | `just check` | ~5s | every change |
| Concurrency (store) | a viewer reading while the books are written sees only whole books, never a false "differs" (2) | `ledger-store/tests/concurrency.rs` | `just check` | <1s | every change |
| Model-based stateful | the Lean state machine, transcribed (3) | `tests/model.rs` | `just check` | ~3s | every change |
| Differential: Decimal vs i64 | the kernel at two types (3) | `kernel_bridge.rs` | `just check` | <1s | every change |
| Differential: GnuCash | another implementation, adjudicated (3) | `tests/oracle_gnucash.rs`, `oracle/gnucash/` | `just oracle` | ~2 min | weekly |
| Formal proofs | Kani (bounded) and Lean (unbounded) | `src/proofs.rs`, `verify/lean/` | `just prove` | ~3 min | weekly |
| Fuzzing | crash, hang, broken books (4); for `fuzz_store_replay` (validly chained journals with fuzz-chosen events): books that open must verify, and the viewer must accept exactly what the writer does | `fuzz/fuzz_targets/`, corpora in `fuzz/corpus/` | `just fuzz <target> <secs>` | 10 min each | weekly |
| Miri | undefined behavior in our use of dependencies (4) | the core's unit, bridge, invariant tests | `just miri` | ~2 min | on demand |
| Golden | `tests/golden/fixture_company.json` (5) | `invariants.rs` | `just check` | <1s | every change |
| Benchmarks | a baseline (6) | `ledger-core/benches/books.rs`, `ledger-store/benches/store.rs` | `just bench` | ~2 min | none (no budget, by decision) |
| Mutation | can the tests fail? | `ledger-core` (2026-10-13: 1,011 caught + 13 timeouts of 1,024 viable, 0 missed) | `just mutants` | ~3 h | weekly |
| Mutation (store) | can the tests fail? | `ledger-store` (2026-10-13: 357 caught + 1 timeout of 365 viable, from 82%; three of its seven survivors then killed; four equivalent left: `sort_value`'s two arms, `OpenFlags` `|` vs `^`, the long form of a sweepless close) | `just mutants-store` | ~12 min | weekly |
| Mutation (import) | can the tests fail? | `ledger-import` (2026-10-13: from 91 of 171 (53%) to 208 of 239 viable on the full run; the 31 survivors (XLSX layout, borders, alignment, exact-text grouping, IIF transliteration, tax-summary optional sections, day-equals-month dates) then killed by targeted tests, re-run clean but one: a blank amount cell written unbordered on an unruled row, invisible, so equivalent; 2026-10-19, the first scheduled run in CI: 375 tested, 327 caught, 42 unviable, 6 missed (the tax-basis rule marks, a heading's alignment, the totals row's blank cells, and that blank cell, after all: a ruled row writes it, an unruled one writes none), all six then caught by `rule_marks_match_their_notes`, `asset_register_xlsx_headings_and_totals_rule` and `xlsx_missing_amounts_are_cells_only_on_ruled_rows`, each checked against its mutant by hand) | `just mutants-import` | ~2 h | weekly |
| Mutation (roles, reload) | can the tests fail? | control roles and settings reload (2026-10-15, targeted: the role paths in `ledger-core` 30 caught of 30 viable; `ledger-store`'s journaling and replay of `role.assigned` 20 of 20; `ledger-mcp` settings, reload, roles, digest 28 of 28) | `cargo mutants -F` on those functions (see the commit) | ~15 min | with the crate's weekly run |
| Mutation (config) | can the tests fail? | `ledger-config` (2026-10-14: 55 caught of 55 viable) | `just mutants-config` | minutes | weekly |
| Mutation (MCP) | can the tests fail? | `ledger-mcp` (2026-10-14: from 197 caught + 5 timeouts of 328 viable (62%) to 314 caught + 2 timeouts of 331; of its 15 survivors, five then killed by targeted tests (the settings file beside the books, year earnings with an open period and with last year's partial close, year 1, a failed flush), one dead guard removed (`profit_and_loss_by_class` skipping placeholders, which never take postings), nine equivalent: `money` rescaling an amount already at two places, the lock wait's deadline instant, `year_earnings` starting the day the year starts either way, the bank import's rehearsal sides (a line's side can't make a draft refuse) and its zero-amount side (zero rows are dropped by the parser), a disposal's zero gain adding zero, and the CPA package index summing debits instead of credits (entries balance)) | `just mutants-mcp` | ~1 h | weekly |
| Mutation (GUI) | can the tests fail? | `ledger-gui` view model and formatting (2026-10-14: 140 caught of 176 viable on the full run, then 92 of 99 on a targeted rerun of the money paths after new tests; of its 7 survivors, one then killed by a test (burn over exactly one month, confirmed by a rerun of `compute_kpis`'s 48 mutants), two dead arms removed (`parties`' bill arms: the ledger tags bill entries with their vendor, which wins), four equivalent: an entry's debits vs its credits (they balance), the three-month burn boundary (both paths give 3), burn over zero covered months (only when the first entry is after the window, which then holds no loss), and `compact`'s sign test at zero (zero prints "$0" unsigned either way)) | retired 2026-10-18 (owner: it crushed the machine; for the viewer, test its two musts directly: it never writes the books, `tests/read_only.rs`, and every figure it shows is the engine's, `tests/view_model.rs`) | | never |

CI: `.github/workflows/check.yml` runs `just check` on every push;
`scheduled.yml` runs proofs, the oracle, fuzzing, and mutation weekly. Mutation is split into shards, one job each (`just mutants-mcp 3/16` runs the fourth of sixteen), because GitHub stops a job at six hours and a whole crate takes longer on its runners (2026-10-19, the first scheduled run: the core and MCP runs were stopped at the limit). Test timeouts are three times each run's own baseline, never under the recipe's floor, so a slower runner doesn't report false timeouts. The viewer has no mutation job.

Seeds and failure databases: proptest's `*.proptest-regressions` files
are checked in and replay on every run. The GnuCash oracle's books come
from fixed seeds (`SEED` in `oracle_gnucash.rs`). Fuzz corpora live in
`fuzz/corpus/<target>`.

Golden review: `fixture_company.json` is hand-verified (docs/oracle.md,
Layer 1). The van depreciation golden was replaced on 2026-10-13 by a
hand computation (`van_schedule_is_the_hand_computed_one`) after every
row was checked against it.

## P0 guard rails (2026-10-16)

Laws added with P0 (`plan/served.md`) and its review, each with the test
that would fail if it broke:

| Law | Test |
|---|---|
| Books from a newer wanderbooks are "newer", never "corrupt", from the writer, the viewer and the server; damage stays corruption (rules.md B-27) | `ledger-store` `tests/format.rs` (`old_reader_gets_newer_journal`, `damaged_journals_are_still_corrupt`); `ledger-gui` `tests/newer_journal.rs`; `ledger-mcp` `tests/startup.rs` |
| A line the gate refuses is called newer only once the next line's `prev` checks its hash; a last line is "newer or damaged" | `a_damaged_line_that_looks_newer_is_corrupt_when_the_next_line_checks_it`, `an_unreadable_last_line_is_reported_as_newer_or_damaged`, `a_newer_line_the_next_line_chains_onto_is_newer_and_intact` (the message vouches for that line only) |
| The envelope and its hash are frozen across formats: a next line in a newer format (a float, a string seq, keys of its own) settles by its `prev` alone, never as corrupt; a key beside the envelope is newer, never dropped, and still checked through; a refused line followed by an unreadable one is "newer or damaged" | `format.rs` `a_newer_format_frames_its_lines_its_own_way`, `an_unknown_envelope_key_is_newer_and_checked_through`, `a_refused_line_followed_by_a_garbled_line_is_unverified` |
| The writer and the viewer read bytes alike: a line that isn't UTF-8 is corrupt to both; an older version never repairs (truncates) newer books | `a_line_that_is_not_utf8_is_corrupt_for_the_viewer_and_the_writer`, `an_old_reader_never_repairs_newer_books` |
| A newer format is newer whatever its features look like; a blank line is damage to the viewer as to the writer | `format.rs` `a_newer_format_is_newer_whatever_its_features_look_like`; `a_blank_line_is_corrupt_for_the_viewer_and_the_writer` |
| A request that panics stops the server: every later request, on every session, is refused | `server.rs` `a_request_that_panics_stops_the_server` |
| No input reaches a panic (each would stop the server for every session): every tool with arguments generated from its schema and made hostile (wrong types, u64::MAX, decimals of 30+ digits or absurd exponents, dates like 0000-00-00, 99999-12-31, +262142-12-31, 2026-02-30, periods like 999999-01, control and format characters, long strings), every resource URI generated; the inputs that once panicked, pinned; after each property the server restarts and the books reopen (a call a command accepts but replay refuses would brick them); every tool and resource over every old book (rules.md B-28) | `ledger-mcp` `tests/hostile_args.rs` `hostile_tool_arguments_never_stop_the_server` (with a floor: most calls reach a tool, some succeed), `hostile_resource_uris_never_stop_the_server`, `inputs_that_once_panicked_are_refused_or_answered`, `old_books_answer_every_tool`; `MCP_CASES` and `OLD_BOOKS_CASES` (default 3 per tool per book) set the cases |
| Dates are in 0001-01-01 through 9999-12-31 and amounts at most a line's, for every new command; replay accepts every line an earlier version accepted (each bound has its pair: the command refuses past it, the replayed old line is accepted); a figure derived past exact arithmetic or chrono's calendar is refused, never a panic; a recurring run makes at most 1,000 drafts (rules.md B-28) | `ledger-core` `tests/spec_bounds.rs`, `calendar.rs` tests, `plans_at_the_edges_are_computed_or_refused`, `intervals_reach_chronos_last_day`, `a_vendor_part_is_scaled_exactly_or_refused`, the `share_property` cases; `ledger-store` `tests/bounds.rs`, `tests/store.rs` (the old-books tests); `server.rs` `periods_are_a_year_and_a_month`, `dates_are_four_digit_years_and_real_days` |
| Opening the books changes nothing until every complete line passes in memory (chain, gate, first line, replay); the viewer applies the writer's first-line rule (rules.md B-27) | `format.rs` `a_bad_complete_line_before_a_torn_tail_leaves_the_journal_as_it_was`, `the_first_line_is_store_created_for_the_viewer_and_the_writer`; `fuzz_store_replay` (the first line varies) |
| A path with a `.` or `..` name is refused as missing before anything is looked up; a root inside the books, the data dir or the vault is refused at startup (dropped as the default) | `local_files.rs` `a_dot_or_dotdot_name_never_tells_whether_folders_outside_exist`, `a_root_holding_the_books_or_the_home_folder_is_refused`; `startup.rs` `an_import_root_holding_the_books_or_the_home_folder_is_refused`, `a_default_download_dir_holding_the_books_is_dropped_not_fatal` |
| UTF-16 text without a byte-order mark is scanned like any text; a refused `books.set_w9` marks no file; a refused resource read never echoes a TIN | `tin_files.rs` (tests) `utf16_text_without_a_byte_order_mark_is_scanned_either_way_round`, `a_refused_set_w9_never_marks_its_file`, unit test `only_nul_interleaved_bytes_read_as_utf16_without_a_mark`; `spec_vault.rs` `resource_refusals_never_echo_a_tin` |
| idempotentHint only where a repeat with the same arguments leaves the journal and the vault as they were; every tool's hints pinned in one table | `spec_vault.rs` `idempotent_tools_change_nothing_when_repeated`; `annotations_complete` |
| books.settings lists every receipt-required and exempt account, uncut; a fresh company is "a new company"; the lint catches `just` however it ends and where the books are | `mcp_stdio.rs` `every_receipt_account_left_out_of_the_connect_text_is_in_books_settings`; `handbook.rs` `a_fresh_company_is_a_new_company`; `tool_text_lint_catches_helpers_and_locations` |
| A local file is read only from inside an import root, never a sealed place (data dir, vault, keys, the key folders that don't hold a root), never through a swap after the check (a component walk with `O_NOFOLLOW`, no step onto a protected inode); a missing, dangling or outside path gets one refusal | `local_files.rs` `import_local_file_refuses_key_and_journal`, the property `import_allowed_iff_regular_file_inside_a_root_outside_the_books`, `refusals_never_tell_what_is_there`, `a_file_swapped_after_the_check_is_never_read`, `a_folder_swapped_after_the_check_is_never_walked_through` (the reviewer's race, without `/proc`), `resolved_follows_every_link_a_dangling_one_too`, `sealing_a_key_never_seals_a_whole_root`; through the tools, `mcp_stdio.rs` `import_local_file_and_set_w9_refuse_the_key_vault_and_journal_inside_a_root` |
| No import root is `/`, the home folder or holds it, or holds the books, data dir, vault or a key; the default is a real downloads folder or none | `local_files.rs` `the_default_root_is_never_the_home_folder_or_the_filesystem`, `a_root_holding_the_books_or_the_home_folder_is_refused`; `startup.rs` `an_import_root_holding_the_books_or_the_home_folder_is_refused`, `a_download_dir_of_home_never_opens_the_home_folder` |
| A file with full TINs is never written into an import root, and never filed into the books: every file the server read or wrote with TINs (from any session, after a restart, out of a CPA package), by every filing tool | `mcp_stdio.rs` `tin_bearing_files_never_land_where_they_could_be_filed_back_in`; `tin_files.rs` (tests) `a_tin_bearing_file_is_refused_from_every_session_and_after_a_restart`; unit test `a_recorded_tin_file_is_refused_by_every_reader_of_the_registry` |
| Text filed into the books is refused when it holds a TIN the vault keeps (nine digits, 3-2-4 by hyphen, space or dot, 2-7 by hyphen or space, anywhere) or a stand-alone printed SSN or EIN (123-45-6789, 12-3456789), and only then: a bank export (routing numbers, long references, a date next to an amount) and a longer number holding an SSN shape go in; a locked vault leaves the printed shapes (rules.md B-26) | `tin_files.rs` (tests) `text_with_a_vault_tin_or_a_printed_one_is_refused_and_a_bank_export_is_not`; unit tests `text_holding_a_vault_tin_is_refused_in_every_rendering`, `a_printed_ssn_or_ein_is_refused`, `a_longer_number_holding_a_tin_shape_passes`, `an_ordinary_bank_export_passes`, `binary_bytes_are_not_scanned` |
| Filing is no TIN oracle: text with more than two TIN-shaped numbers ending in a known TIN's last four (the books' TIN and W-9 facts, then the vault's own) is refused before the vault is opened, byte for byte alike whether one is the vault's TIN or none is, the vault open or locked, by every filing tool; one or two such numbers are compared against the vault, one number written many ways counts once, and other last fours never count (rules.md B-26) | `tin_files.rs` (tests) `text_probing_for_a_tin_is_refused_alike_by_every_filing_tool`; unit tests `text_probing_for_a_tin_is_refused_alike_whether_or_not_it_holds_it` (the vault never opened for a probe), `one_or_two_candidates_are_compared_against_the_vault` |
| The connect text fits its budgets, its lead stands alone, it states the receipt rule enforced (exact threshold in the base currency, required and exempt accounts, reversals) and the parties rule in force, and the company name can't break it (control and format characters) | `handbook.rs` `instructions_budget`, `a_typical_company_gets_the_whole_connect_text`, `the_company_name_is_one_plain_line`; `receipt_policy.rs` `the_rule_told_is_the_rule_enforced`, `a_long_or_odd_account_list_stays_short_and_plain`; a new session is told the settings in force: `a_new_session_is_told_the_settings_in_force` |
| No text the agent is given (whole tool JSON, resources, connect text, handbook, every error template and refusal) points at files, git, GitHub, repo docs, flags or helper commands; descriptions fit 700 characters (listed exceptions, P5) | `tool_text_lint`, `tool_descriptions_fit_700_characters`; `ledger-core` `an_unknown_year_threshold_is_never_guessed` |
| Every tool is annotated (openWorldHint false on all), and the read-only hint is true in behaviour (journal, every data-dir file's contents, the database's rows, the vault and the settings unchanged) | `annotations_complete`; `spec_mcp.rs` `read_only_tools_change_nothing` (checked on every call in `Mcp::tool`) |
| A bare (or empty-`since`) digest covers this month so far, on any day of it, as the server counts it (the stdio test reads the server's own month, and starts over if it turns) | `a_bare_digest_covers_this_month_so_far`; `mcp_stdio.rs` `digest_resource_bare_reads` |
| The handbook: every section, "all" in order, the schema names every id | `handbook_sections_render_the_rule_in_force`, `the_whole_handbook_is_every_section_in_order`, `mcp_stdio.rs` `handbook_tool_and_resources_over_stdio` |

## P0 mutation testing (2026-10-16)

Every mutant cargo-mutants makes in the final P0 diff (`jj diff -r
'main..@'`, `--in-diff`), per crate, against that crate's tests (the
recipes' settings: `STORE_CASES=64`, `MCP_CASES=16`, no shrinking; the
viewer against its lib tests and `newer_journal`). Final run:

| Crate | Mutants | Caught | Timeouts | Unviable | Missed |
|---|---|---|---|---|---|
| `ledger-mcp` | 296 | 208 | 3 | 71 | 14 (equivalent, below) |
| `ledger-store` | 73 | 58 | 0 | 12 | 3 (equivalent, below) |
| `ledger-config` | 35 | 35 | 0 | 0 | 0 |
| `ledger-gui` | 9 | 6 | 0 | 3 | 0 |
| `ledger-core` | 1 | 1 | 0 | 0 | 0 |

(`ledger-cli`'s change makes no mutant.) After the filing scan's probe
bound (rules.md B-26), `tin_files.rs` alone was run again against the
whole P0 diff: 74 mutants, 74 caught, among them every mutant of the
bound (`probes` made true or false, `>` made `>=`, `==` or `<`, the
last four's `%` made `/` or `+`, here and for the vault's own last
fours) and of the shared `tin_shaped` scan. The three MCP timeouts are
`resolved` without its hop limit (`< 40` made true, the count made to
go down or stay 0): a link loop then never ends, so the tests can't
pass. The first run missed 36 in the MCP server, 5 in the store, 4 in
config and 1 in the viewer. Each was killed by a test of the law it
broke, removed by simpler code, or is equivalent:

- **Killed (MCP).** A section's title is its heading
  (`a_title_is_the_heading_text`; over stdio the handbook tool titles a
  section by its heading and "all" as the whole handbook,
  `handbook_tool_and_resources_over_stdio`). A refused default import
  root is dropped, never fatal (`ImportRoots::explicit`:
  `a_default_download_dir_holding_the_books_is_dropped_not_fatal`). A
  `.jpg` or `.jpeg` W-9 is taken (`every_w9_file_type_is_taken`). The
  waker wakes: a request pending until another thread wakes it completes
  (`a_request_that_waits_is_woken_and_completes`). The walk opens each
  folder with `O_DIRECTORY | O_NOFOLLOW`: a folder swapped for a link to
  an unprotected folder outside the roots, or for a pipe, is never walked
  through and never blocks
  (`a_folder_swapped_for_a_link_or_a_pipe_is_never_walked_through`). A
  read failure names its kind, never the path
  (`cant_read_names_the_failure_not_the_path`). `resolved` follows 40
  links, as the kernel does, and no more
  (`resolved_follows_forty_links_and_no_more`). A file is saved only to
  a new file in a folder that exists, and a missing folder or a taken
  file is refused as such, never as a failed write
  (`a_package_goes_only_to_a_new_file_in_a_folder_that_exists`,
  `the_w9_file_goes_to_the_vault_never_the_documents`). The CPA package
  marks only its own TIN files: a statement it carries still goes in
  afterwards (`a_tin_bearing_file_is_refused_from_every_session_and_after_a_restart`).
  An unreadable TIN-file registry refuses every file
  (`an_unreadable_registry_refuses_every_file`). Text that isn't UTF-8
  is scanned when its type is `text/*`, a JSON type (`/json`, `+json`)
  or its name says text (`encodings` in the `tin_files.rs` unit tests).
- **Killed (store).** `read_journal_and_format` returns every line and
  the format in force (`read_journal_and_format_gives_every_line_and_the_format`).
  The newline that ends a line is no part of it, so the viewer's words
  about a damaged line are the writer's, opening and following
  (`the_viewer_and_the_writer_say_the_same_of_a_damaged_line`; it kills
  `snapshot.rs` `raw.len() - 1` → `/ 1`).
- **Killed (config).** The short rule (no codes listed) counts each kind
  of account, whichever kinds there are, and says nothing of accounts
  when there are none (`the_short_rule_counts_each_kind_of_account`); a
  code of exactly 12 characters is kept whole
  (`a_long_or_odd_account_list_stays_short_and_plain`).
- **Killed (viewer).** The journal page tells a `journal.format_set`
  line (`a_format_this_version_reads_opens_and_the_journal_page_tells_it`).
- **Simpler code (MCP).** `inspect` compared `canonicalize` with
  `resolved`; only a race between those two calls could make them differ,
  and the file read was the checked one either way. It now places an
  existing path by `canonicalize` itself. `read_checked` refused by the
  file's size before reading, so its read limit (`limit + 1`) was only
  reached by a file growing during the read; the read's limit is now the
  one check (the 25 MiB upload test kills `+ 1` → `* 1`). `openat` failed
  on `fd < 0`, now on `fd == -1`, as openat(2) specifies.
- **Equivalent (MCP, all 14 missed).** `^` for `|` in the open flags
  (six: the walk's first step, each folder's two, the file's two, and
  `openat`'s `O_CLOEXEC`): `O_RDONLY` is 0 and `O_DIRECTORY`, `O_NOFOLLOW`,
  `O_NONBLOCK`, `O_CLOEXEC` are distinct bits, so `a ^ b` is `a | b`.
  `&` for `|` in the first step's `O_RDONLY | O_DIRECTORY`: it opens `/`
  read-only either way, and `/` is always a folder. The off-Unix stubs
  (`Protected::inodes`, five mutants; `open_walk`; `hard_linked`) are
  not compiled on Linux, so their mutants change nothing here.
- **Equivalent (store, all 3 missed).** `Snapshot::journal_format` and
  `Store::journal_format` returning the default: this version reads
  format 1 with no features only (`SUPPORTED_JOURNAL_FORMAT` 1,
  `KNOWN_FEATURES` empty), and that is the default, so no journal it
  opens has another format in force; the first format 2 or feature makes
  them killable. `Store::open` setting aside a torn tail when
  `complete <= raw.len()`: `repair_torn_tail` re-reads the journal under
  the writer lock and does nothing when it ends in a newline.

### Round 3 (2026-10-17)

`cargo mutants --in-diff` on this round's changes (the diff from the
round-2 stack to round 3), per crate, output under
`~/.cache/wanderbooks/mutants/p0-r3-<crate>` (`MCP_CASES=16`,
`STORE_CASES=64`, `PROPTEST_MAX_SHRINK_ITERS=0`):

| Crate | Mutants | Caught | Missed, then killed | Equivalent | Timeouts | Unviable |
|---|---|---|---|---|---|---|
| `ledger-core` (tests of core and store) | 63 | 53 | 3 | 1 | 0 | 6 |
| `ledger-store` | 40 | 34 | 0 | 0 | 2 | 4 |
| `ledger-mcp` | 85 | 63 | 5 | 1 | 0 | 16 |
| `ledger-config` (a doc comment only) | 0 | | | | | |

- **Killed (core).** `replay_add_line_fx`'s size check (`>` to `==` or
  `>=`) and `validate_recurring`'s (`>` to `>=`): a line, base or
  foreign, added or replayed, and a template line may be exactly the line
  limit (`spec_bounds.rs` `the_bound_is_the_line_limit_itself`).
- **Timeouts (store, counted as caught).** `run_recurring`'s cap (`>` to
  `==` or `<`): without it, a monthly entry run through 9999-12-31 writes
  some 95,000 drafts, which is what the cap is for
  (`bounds.rs` `a_recurring_run_has_an_end`).
- **Killed (MCP).** `text_of`'s byte-order-mark arms: text with a mark and
  no NUL byte at all (full-width digits, CJK) is still read as UTF-16
  (`utf16_with_a_mark_is_read_whatever_it_holds`); its NUL-stripped
  reading (`!=` to `==`): NULs scattered in text hide no TIN
  (`nuls_scattered_in_text_hide_no_tin`); `nul_interleaved`'s eighth
  (`*` to `+`): half NUL on one side and three of sixteen on the other is
  not UTF-16; `redact_value`'s array arm: a TIN inside an array of an
  error's data is masked (`redact_error_masks_every_tin_in_the_message_and_data`).
- **Equivalent.** `k1::share_property` returning early on
  `whole <= 0 && total <= 0` instead of `||`: `whole` is
  `min(pool, total)` with both sums of non-negative amounts, so the two
  differ only when `whole` is 0 and `total` positive, where the
  computation goes on to split 0 into zeros, the early answer.
  `redact_value`'s guard `has_full_tin(t)` replaced by `true`:
  `redact_full_tins` returns its input unchanged when it finds no run,
  exactly when `has_full_tin` is false, so the guard only saves a copy.

### Round 4 (2026-10-17)

`cargo mutants --in-diff` on this round's changes (replay restored to
main's rules, derived figures checked, the old-books contracts), per
crate, output under `~/.cache/wanderbooks/mutants/p0-r4-<crate>`
(`MCP_CASES=16`, `OLD_BOOKS_CASES=1`, `STORE_CASES=64`,
`PROPTEST_MAX_SHRINK_ITERS=0`; the GUI's old-books test over the edge
books only, `OLD_BOOKS_ONLY=x_draft_max_lines,x_schedule_chrono_end,...`,
since `asset_long_life` alone takes three minutes a run):

| Crate | Mutants | Caught | Missed, then killed | Equivalent | Timeouts | Unviable |
|---|---|---|---|---|---|---|
| `ledger-core` (tests of core and store) | 176 | 153 | 6 | 3 | 0 | 14 |
| `ledger-store` | 12 | 11 | 0 | 0 | 0 | 1 |
| `ledger-mcp` | 81 | 13 | 5 | 1 | 0 | 62 |
| `ledger-config` | 7 | 7 | 0 | 0 | 0 | 0 |
| `ledger-gui` | 221 | 47 | 137 | 11 | 7 | 19 |

- **Killed (core).** `name_payroll_parties` (replaced by `Ok(())`, and
  `==` to `!=` in merging one employee's lines): a payroll entry names
  its employees, each with their net pay, live and on replay
  (`spec_parties.rs` `a_payroll_entry_names_the_employees_it_paid`);
  `entry_flow`'s `across_left -= take` (`+=`, `/=`): across is used up
  in order (`entry_flow_takes_across_in_order_until_used_up`);
  `share_target`'s debit filter (`==` to `!=`): an unbalanced draft
  moving no money splits over its debits
  (`a_share_target_without_money_is_the_debits`); `resolve`'s capital
  debit sign: members' derived shares net their capital lines
  (`derived_member_shares_net_their_capital_lines`). These were in code
  this round touched only to make it checked; the core and store tests
  never reached them (the MCP tests did).
- **Killed (MCP).** `lines_settled` (whole and in parts): an empty, an
  unbalanced and a Decimal-overflowing draft are not settled, a
  balanced draft and a posted entry are (`server.rs`
  `settled_lines_are_posted_or_balanced_drafts`).
- **Killed (GUI).** The pages' display of what can't be computed and
  of the edges was barely asserted: new tests in `tests/edges.rs` and
  beside the pages (`test_support.rs` renders a page headless and reads
  its painted text and shapes) cover the runway and "coming up" on the
  cash page, the balance chart's labels and hover, an entry's totals
  (balanced, out of balance, too large to total), a party's span and
  monthly chart at chrono's edges, a split entry's part, the register's
  reconciliation colours, the company page's cards, the report range,
  prior period and column limit, and the transaction row. The rerun
  (`p0-r4-gui-rerun`) left only the equivalents.
- **Equivalent (GUI, 11).** `party_span`'s `<` to `<=` (twice): they
  differ only at year 0, which chrono holds, so the fallback is never
  taken. The asset notice's `>` to `>=`: with nothing unposted both
  amounts are 0 and nothing is pushed. `occurrences_from`'s five
  arithmetic mutants: each leaves `k0` a lower bound on the first
  occurrence on or after `from`, so only the walk's length changes. The
  cash forecast's `>` to `>=`: a zero amount added to inflow instead of
  subtracted from outflow. The company page's `total.is_zero()` guard:
  a schedule's total is always positive. Its recurring total's `==` to
  `!=`: a template always balances, so its credits sum to its debits.
- **Equivalent.** `entry_flow`'s filters `v > 0` to `>= 0` and `v < 0`
  to `<= 0`: they only add zeros to a sum. `split_first`'s
  `weights.is_empty() || total.is_zero()` to `&&`: every weight is
  positive (a percent above 0, a member's share checked above 0), so a
  non-empty list never sums to zero. `lines_settled`'s side filter
  `l.side == s` to `!=`: it swaps which sum is called debits, and the
  comparison and the overflow refusal are symmetric.
- **A real defect found while writing these tests:** the depreciation
  true-up named its final month with chrono's `%Y`, which signs a year
  past 9999 ("+10004-03"), while the lines don't, so an asset whose life
  ran past 9999 never trued up and could depreciate past its salvage
  (`spec_replayed_extremes.rs` `a_life_past_9999_trues_up_to_the_cent`).

## P1: the books keep how they are kept (2026-10-17)

| Law | Test |
|---|---|
| Replay is pure: after any sequence of settings, convention, decision, open-item, template and approval commands, valid or refused, the live store, the store reopened from its journal and the viewer's snapshot hold the same settings and conventions, and the store verifies (database row for row) | `ledger-store` `tests/settings.rs` `settings_replay_is_pure` (proptest, 512 cases per check; 2,000 by hand, release, passed) |
| The first such change declares journal format 2, once; a change to what is set journals nothing; refused changes journal nothing | `the_first_change_declares_format_2`, `refused_changes_journal_nothing` |
| Approval rules block nothing until enforced; then an entry a rule covers posts only once its approver approved it as it stands | `approval_rules_off_by_default`; `ledger-mcp` `spec_kept.rs` `approval_rules_through_the_server` |
| A template drafts exactly, odd cents by largest remainder, its slots named by each draft, all or nothing | `templates_draft_exactly_or_not_at_all`, `template_slots_take_the_account_each_draft_names`, `templates_draft_through_the_server`, core `splits_are_exact` |
| Books written under P1 open to the same record forever | `books_written_under_p1_open_to_their_record` (fixture `old-books/p1_record`, also in every old-books test of the store, the viewer and the server) |
| A settings file imports exactly, reads as it did, and an export round-trips; a masked EIN never imports | `ledger-config` codec unit tests, `tests/codec_props.rs` `exports_round_trip` (512 cases), `ledger-cli` `the_toml_and_its_examples_import_and_round_trip` |
| Moving settings in: set refused before, the preview records nothing, the file never read after (not even to start), an import that reads differently or loses the vault key refused, the EIN only ever masked | `spec_kept.rs` `a_settings_file_moves_into_the_books`, `an_import_that_reads_differently_is_refused`, `an_import_needs_the_vault_key_on_the_command_line` |
| books.start fits in 8 KB and says what it cut | `start_fits_in_8_kb` |
| Values are canonical; no full TIN but the company's EIN; settings hold together; replay takes shapes | core `settings::tests` |
| Every new tool on the happy path, schema-checked; read-only tools change nothing; idempotent tools repeat as no-ops | `spec_mcp.rs` `inv1_*`, `read_only_tools_change_nothing`; `spec_vault.rs` `idempotent_tools_change_nothing_when_repeated` |
| The connect text names books.start in its lead and where settings live, in budget, journaled or not | `instructions_budget`, `a_typical_company_gets_the_whole_connect_text` |
| Fuzzed journals of the new events open in the writer and the viewer alike, verify, and read every way without a panic | `fuzz_settings_replay` (and `fuzz_store_replay` with the new types) |
| A record changes no figure | Lean `Step.record_changes_no_figure`, `Reachable.records` |

### P1 mutation testing (2026-10-17)

Every mutant in the P1 diff (`--in-diff`), per crate, against that
crate's tests and its nearest callers' (the recipes' settings:
`STORE_CASES=64`, `MCP_CASES=16`, no shrinking). First run, then after
targeted tests:

| Crate | Mutants | First run: caught / timeouts / unviable / missed | Missed after |
|---|---|---|---|
| `ledger-core` (with `ledger-store`'s tests) | 486 | 318 / 5 / 41 / 122 | 1 (equivalent) |
| `ledger-config` (with `ledger-cli`'s) | 72 | 44 / 0 / 2 / 26 | 0 |
| `ledger-store` | 113 | 81 / 0 / 28 / 4 | 1 (equivalent) |
| `ledger-mcp` | 260 | 156 / 0 / 96 / 8 | 0 |
| `ledger-gui` | 25 | 7 / 12 / 3 / 3 | 0 |

No survivor was a bug; each was a test gap, now closed by a test of
the law it broke:

- **Core.** `tests/settings_units.rs`: every settings accessor's exact
  value, the company profile and its order, every party-policy
  spelling, fiscal-year bounds off January, the TIN detector's joiner
  limits, each text, share and month-day boundary, every tax
  classification and the return it names, the one-quadrillion cap,
  today in the company's time zone, and the display of members, lists
  and the masked EIN. `tests/conventions_units.rs`: slot names (32 and
  33 characters), summaries cut at 160 characters, template bounds (2
  and 50 lines kept, 1 and 51 refused, a share out of (0, 1] named by
  its line), the approval fingerprint against an independent FNV-1a of
  the hashed text, approval rules at the limit and one cent over
  (debits only), and which fields make a convention's revision.
- **Store.** An empty import still moves the books in (once); a
  posting template replay can't read is newer.
- **Config.** The TOML comment scanner (a `#` in basic and literal
  strings, an escaped quote), comments carried by an import, a masked
  EIN matching, not matching and into empty books, an empty list
  changing nothing, `differences` naming each field, the export's
  provenance lines, `effective_receipt_policy` with and without
  `config.json` (`tests/effective_policy.rs`).
- **MCP.** `Live::journaled` reads no file, `sync` reloads once per
  change; the books://start, settings and conventions resources; a
  renumber names exactly the conventions that name the old code.
- **Viewer.** The Company page's cards (`tests/kept.rs`): live
  conventions and not retired ones, standing decisions and not
  superseded ones, open items and not resolved ones, who decided each
  and when, a late item saying so in words ("overdue since", added for
  this: colour alone told only the sighted), and the empty-profile copy
  of books that keep their settings. `--vault-key-file` wins over the
  settings and an unknown argument exits 2 (`tests/vault_key_flag.rs`).
  The first run's 12 timeouts were builds slowed by parallel work: the
  rerun caught them all.

Equivalent: `holds_printed_tin` mapping a full-width digit by `/`
instead of `-` (every such digit becomes the digit 1: it asks only
whether a character is a digit), and `approval_gate`'s fast path (`||`
made `&&`: with rules off or none, the probe's `approvals_missing` is
empty either way, so only the cost differs).

**After the cutover (2026-10-17).** Two fixes found by the first
company's P1 cutover, mutation-tested on their diff (27 mutants: 22
caught, 4 unviable, 1 equivalent): the K-1 worksheet prints a share by
its value, at least two places ("50%" and "0.5" alike print "0.50":
`a_share_prints_the_same_however_it_was_written`, and `spec_vault`'s
package test), and the filing scan passes the company's own EIN printed
2-7 and nothing else (`the_company_s_own_ein_printed_as_an_ein_passes`,
`tests/tin_files.rs` `the_company_s_own_ein_goes_in_and_no_other`).
Equivalent: `share_text`'s `scale() < 2` made `<= 2` (padding a
two-place share to two places changes nothing).

## P2 slice 1: one executable (2026-10-18)

`wanderbooks dev-stdio` and `wanderbooks gui` run the server and the
viewer from the one executable (plan D-22); `tests/one_executable.rs`
drives both, and `just server-only` builds, clippies and tests the build
without the viewer and fails if it links a window library. Mutation on
the diff, two jobs, the viewer left out (owner, 2026-10-18): the server's
`run` against the server's tests, 14 mutants, 12 caught or unviable, the
exit text's then caught by a new test, and one equivalent (`<` vs `<=`
on the ten-second lock deadline); the CLI, 24 mutants against its tests
in the default build and the `run_gui` arms again in the server-only
build (`--no-default-features`), every one caught in the build where its
code exists.

## P2 slice 2: the data root (2026-10-18)

`ledger_store::root` (plan 3.4, 5.5) and `wanderbooks setup`:
`tests/root.rs` (32), the module's unit tests (4), `ledger-cli/tests/setup.rs`
(3). Three review rounds (12 findings, then 5, then 1, all fixed).
Mutation on the diff, the viewer left out, two jobs: the CLI, 7 of 7
caught; `root.rs`, 47 missed on the first run, then new tests (the blob
copier's strays, errors other than not-found, the buffer that grows
behind `getpwuid_r`/`getgrgid_r`, the private-group rule) and a rerun:
201 mutants, 163 caught, 23 unviable, 1 timeout (the growing buffer
halved instead of doubled, which never ends), and 14 left, none
reachable by a test:
- the lookups' failure guards (`rc != 0 || null || null` with `&&`,
  five): on every lookup that succeeds they agree, and this account's
  own lookup always succeeds;
- `NotFound` guards whose other errors end the same way (setup's
  `symlink_metadata` of the chosen root, where creating fails with the
  same error; the location note's removal, unreachable after the note
  was checked to be a plain file; a subfolder's lookup inside a root
  already opened);
- `create_book`'s clean-up after `Store::create` fails in a folder it
  just made (no test can make that fail);
- `make_private_dirs`' `AlreadyExists` guard (two setups racing);
- `sync_dir` (durability, not visible to a test);
- the group's member check (`!` deleted), which agrees on every account
  whose own group lists no members, the usual case.

## Viewer color schemes (2026-10-19)

`ledger-gui/src/schemes.rs` (docs/color-schemes.md). No mutation run (the
viewer's rule); its two musts are tested directly instead:
- `schemes::painted::every_scheme_paints_the_same_words_and_figures_readably`
  draws thirteen pages (an entry, the company, assets, documents, overview,
  cash, profit, transactions, the P&L, the balance sheet, accounts,
  parties, the journal) in all 80 schemes, a thread a page, and reads what
  was painted where it can be seen: every run of text, report rows
  included, is the same and in the same place in every scheme, and each is
  at 4.5:1 or better against what was painted under it by then (the fills
  in paint order, cut to their clips, translucent ones laid over what was
  below; every section of a text, its own color or the fallback). It found
  egui's placeholder text (its weak text, 60% alpha of the text) at 2.8:1.
  Not painted by it: hover, selection and new-entry rows, tooltips and
  the menu, whose grounds the palette tests check by arithmetic.
- `the_system_turning_light_or_dark_changes_nothing`: starting as eframe
  does (following the system), a scheme chosen and the system then
  flipped leaves every painted color as it was. It fails on the first
  version, which set only the current theme's style.
- The palette tests check every scheme: every text color at 4.5:1 on
  every ground (soft fills included), text over secondary over faint each
  at least 1.2 times the contrast of the next on its worst ground, chart
  series at 3:1 on cards and the hover tint, the charts' three series shown
  together and gains, losses and text each ΔE 20 or more apart, the
  logo's initial readable on the accent, light or dark as its background
  is, and the filter showing exactly its mode; the menu test chooses,
  filters and keeps a scheme, and its accessible labels check only the
  family in force and select the variant in force; `app::tests` restores
  the scheme (and the old light/dark choice) from the preferences.

The first version's review (2026-10-19) found figures turning unreadable
when the system's theme flipped, Ayu and the families after it out of
reach in a short window, Income and Net the same color in twelve schemes,
pills, faint figures and the logo initial below 4.5:1, a new-entry tint
that bleached reds and greens, and a "same figures" test that compared
only accessibility labels (no report rows); all fixed, with the tests
above. Round 2 found secondary and faint text made alike by lifting each
alone (identical in Wanderbooks Light), the painted test picking a text's
ground without paint order or clips, and every family's accessible label
carrying the hidden check; fixed, with the tests above. The owner, trying
it (2026-10-19): Ayu out of reach (the family list scrolled inside the
menu, its scroll bar hidden), the families not alphabetical, and a click on
a variant's colors doing nothing. Now the families are alphabetical, in
columns with no scrolling, and a variant's whole row is its button;
`every_family_is_on_screen_in_order_and_a_whole_row_chooses` checks all
three in a tall window and a short one. The live preview (owner, the same
day) is checked by `a_hovered_scheme_is_shown_live_and_only_a_click_keeps_it`
with frames 10 ms apart: not worn before 50 ms, worn after, the next row's
scheme on moving, the one in force off the rows and at once on closing,
nothing kept until a click. Earlier: the viewer's own light palette's green below 4.5:1 on the
sidebar (4.35:1), and a first fix (moving colors toward the text color)
that turned Everforest's red and green into the same grey; colors keep
their hue and change only lightness.

## Benchmarks (baseline 2026-10-13)

No budget, by decision (the owner, 2026-10-13: the books do little work
and are fast enough). The numbers are a baseline to spot a regression.

40-account chart, two-line entries across a year, on the development
laptop:

| | 1k entries | 10k entries |
|---|---|---|
| clone the ledger (store rehearsal, per command) | 56 µs | 1.0 ms |
| post one more entry | 1.0 µs | 5.8 µs |
| trial balance | 44 µs | 1.07 ms |
| balance sheet | 47 µs | 1.04 ms |
| one month's P&L | 25 µs | 248 µs |

The store, 10k two-line entries on the laptop's disk (every journal
append is synced; `benches/store.rs` in ledger-store):

| | 10k entries |
|---|---|
| open the books (verify the chain, replay, catch the database up) | 249 ms |
| the viewer loading them | 616 ms |
| a full verify (replay, projection, every table) | 809 ms |
| post one entry (create, two lines, post: four synced appends) | 62 ms |

## Formerly assumed: checked against the standards (2026-10-13)

The owner isn't an accountant and won't adjudicate these; each was checked
against published sources instead (rank: FASB ASC via the Deloitte DART
reproduction, SEC, IRS; then textbooks; software practice is evidence of
convention only). Verdicts: CONFIRMED (a source supports it), CONVENTION
(no standard governs; matches mainstream practice), DESIGN (not an
accounting question), CHANGE (a source says otherwise; work pending).

| # | Behavior | Verdict | Source / note |
|---|---|---|---|
| 1 | A P&L whose start is after its end is refused | DESIGN | No standard governs a reversed range; refusing beats silently swapping a typo |
| 2 | A balance sheet omits zero-balance accounts | CONFIRMED | Reg. S-X 210.4-03(a): "No caption should be shown ... as to which the items and conditions are not present." A comparative statement must still show a zero beside a non-zero prior column |
| 3 | The lifetime `profit_and_loss` counts closing entries | CONFIRMED | Used as earnings not yet closed on the balance sheet; closed income is already in capital (OpenStax Principles of Financial Accounting 5.1). Renamed `unclosed_earnings` (2026-10-14) |
| 4 | Closing a period with nothing to close still locks it | CONVENTION | A close is a lock (B-12); a dormant month must still lock and count for close-in-order |
| 5 | Draws and distributions are closed into capital at period end | CONFIRMED | OpenStax 5.1: dividends "are closed to the permanent account"; drawing accounts close to owner's capital. Monthly closes mean the year's K-1 Item L withdrawals sum across months |
| 6 | Reconciliation balances | CONFIRMED | Signed by the account's normal side (a card reads as owed), as statements show it; `reconciling_a_card_reads_its_normal_side_...` |
| 7 | A member's ending on one equity statement may differ by a cent from the next statement's beginning | CHANGED 2026-10-13 | A capital account is one running balance (Form 1065 K-1 instructions, Item L). Undistributed earnings now split cumulatively as of each date, so each member's capital chains exactly (`each_members_capital_chains_across_statements`; it fails on the old code); rules.md B-13 |
| 8 | Cash flow nets one line per account; investing comes only from registered assets | CHANGED 2026-10-13 | ASC 230-10-45-7: gross receipts and payments "more relevant" than net; 45-8 limits netting to quick-turnover items. Financing and investing are now gross; loans made and investments bought are investing (230-10-20, 45-13) via `[books] investing_accounts`. Owner-advanced expenses: PwC 6.9.3.1 constructive treatment via `[books] financing_accounts` (rules.md B-14) |
| 9 | An asset's life ends at purchase + life months, that day not held; day-count proration | CONVENTION | Exactly N months of service; day proration and the mid-month convention are both accepted book policies (ASC 360-10-35-4, read 2026-10-14: any systematic and rational allocation) |
| 10 | A payroll entry has one line per account total | CONFIRMED | The per-employee detail the IRS requires ("Amounts and dates of all wage ... payments", Employment Tax Recordkeeping) is kept in the run; a summary entry is standard |
| 11 | A period with exactly the dates of an existing one is allowed | DESIGN | Nothing governs duplicate periods; harmless (the second close locks only). Refusing exact duplicates is an option |
| 12 | Each close into members' capital rounds on its own (ties to the earlier member) | CHANGED 2026-10-13 | Twelve monthly 50/50 closes could leave one member several cents ahead. A run of members closes now splits cumulatively (rules.md B-13; Lean `runPart_telescopes`; property `each_member_stays_within_a_cent_of_their_share` fails on the per-close rule); journaled `"cumulative"`, older closes replay as booked |

Questions are settled from the sources, not by any one company's CPA
(wanderbooks is general software). Where the standards leave a real
choice, it is a per-company setting with a sourced default, for that
company's accountant to review: the cash flow account roles (rules.md
B-14) are the first.

## Gap list: what isn't covered, and what would bring it in

| Technique | Status | Trigger |
|---|---|---|
| Fault injection | SKIP for the core (no I/O, clock, or network); DONE for the store's journal (full disk, cut-off append) | the first I/O or clock in `ledger-core` |
| Integration | SKIP for the core (no collaborators) | the core gains one; covered at the store and MCP layers |
| End-to-end | SKIP for the core (a library) | MCP stdio tests cover the journeys |
| Security properties | SKIP for the core (no roles, no secrets) | the first permission concept, or a secret-bearing field |
| Concurrency stress | SKIP for the core (no shared state); DONE for the store's writer + viewer (found the false "differs") | the first `Arc`, atomic, or lock in the core; the MCP server's HTTP transport shares one store across sessions (next) |
| Lean: apportion's sum law for any n | DONE (2026-10-14) | verify/lean/Wanderbooks/Apportion.lean: ranks are a permutation of 0..n-1 (strict total order + pigeonhole, core Lean only), so exactly u parts get a unit and one the fraction; `apportion_sum` and `apportion_sum_of_shortfall`, standard axioms only |
| ASC paragraphs not yet read | DONE (2026-10-14) | read in the ASC basic view (`docs/spec/sources/fasb-asc.md`): all confirm the rules; one citation fixed (230-10-45-14(b), not (2)) |
| GnuCash's own close | OPEN | the oracle books closes as ordinary entries; comparing GnuCash's Close Book needs its Scheme API |
| Import: spec-blind suite, footing properties, mutation | DONE | sources vendored (Intuit bank CSV and IIF, ECMA-376/Excel numbers, IRS form lines); specs on every module; tests/spec_import.rs (spec-blind, footing and round-trip properties, XLSX read back from its XML, layout and styles); fuzz_import_csv; templates checked against the engine's control accounts; mutation one equivalent left |
| MCP, config, GUI crates | DONE (2026-10-14) | the same playbook: sourced module specs (config: Pub 538, Treas. Reg. 301.7701-3; MCP: the MCP tool spec and JSON-RPC 2.0); spec-blind suites `spec_config.rs`, `spec_mcp.rs`, `spec_gui.rs`; the MCP oracle `oracle_mcp.rs` (every export against ledger-core); startup, HTTP concurrency, and the viewer's read-only promise (`read_only.rs`) tested; `fuzz_config_parse`; mutation above |
