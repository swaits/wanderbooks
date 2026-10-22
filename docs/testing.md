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
| Formal proofs | Kani (bounded) and Lean (unbounded) | `src/proofs.rs`, `verify/lean/` | `just prove` (48 harnesses and Lean), `just prove-slow` (the 8 in the justfile's `kani_slow`, each under its own 30-minute cap) | prove ~5 min and prove-slow ~8 min at four jobs on a quiet laptop (each ~25 min under heavy load; docs/verification.md, "Running it") | weekly (CI runs both) |
| Fuzzing | crash, hang, broken books (4); for `fuzz_store_replay` (validly chained journals with fuzz-chosen events): books that open must verify, and the viewer must accept exactly what the writer does | `fuzz/fuzz_targets/`, corpora in `fuzz/corpus/` | `just fuzz <target> <secs>` | 10 min each | weekly |
| Miri | undefined behavior in our use of dependencies (4) | the core's unit, bridge, invariant tests (the exhaustive calendar walk aside) | `just miri` | ~4 min | on demand |
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
| UTF-16 text without a byte-order mark is scanned like any text; a refused `books.set_w9` marks no file; a refused resource read never echoes a TIN | `tin_files.rs` (tests) `utf16_text_without_a_byte_order_mark_is_scanned_either_way_round`, `a_refused_set_w9_never_marks_its_file`, unit test `only_nul_interleaved_bytes_read_as_utf16_without_a_mark`; `spec_vault.rs` `resource_refusals_never_echo_a_tin` |
| idempotentHint only where a repeat with the same arguments leaves the journal and the vault as they were; every tool's hints pinned in one table | `spec_vault.rs` `idempotent_tools_change_nothing_when_repeated`; `annotations_complete` |
| books.settings lists every receipt-required and exempt account, uncut; a fresh company is "a new company"; the lint catches `just` however it ends and where the books are | `mcp_stdio.rs` `every_receipt_account_left_out_of_the_connect_text_is_in_books_settings`; `handbook.rs` `a_fresh_company_is_a_new_company`; `tool_text_lint_catches_helpers_and_locations` |
| A file with full TINs is never written to a path (handed over instead, D-29), and never filed into the books: every file the server read or wrote with TINs (from any session, after a restart, out of a CPA package), by every filing tool | `mcp_stdio.rs` `tin_bearing_files_are_refused_when_given_back_to_be_filed`; `spec_vault.rs` `a_package_is_handed_over_and_never_written`; `tin_files.rs` (tests) `a_tin_bearing_file_is_refused_from_every_session_and_after_a_restart`; unit test `a_recorded_tin_file_is_refused_by_every_reader_of_the_registry` |
| Text filed into the books is refused when it holds a TIN the vault keeps (nine digits, 3-2-4 by hyphen, space or dot, 2-7 by hyphen or space, anywhere) or a stand-alone printed SSN or EIN (123-45-6789, 12-3456789), and only then: a bank export (routing numbers, long references, a date next to an amount) and a longer number holding an SSN shape go in; a locked vault leaves the printed shapes (rules.md B-26) | `tin_files.rs` (tests) `text_with_a_vault_tin_or_a_printed_one_is_refused_and_a_bank_export_is_not`; unit tests `text_holding_a_vault_tin_is_refused_in_every_rendering`, `a_printed_ssn_or_ein_is_refused`, `a_longer_number_holding_a_tin_shape_passes`, `an_ordinary_bank_export_passes`, `binary_bytes_are_not_scanned` |
| Filing is no TIN oracle: text with more than two TIN-shaped numbers ending in a known TIN's last four (the books' TIN and W-9 facts, then the vault's own) is refused before the vault is opened, byte for byte alike whether one is the vault's TIN or none is, the vault open or locked, by every filing tool; one or two such numbers are compared against the vault, one number written many ways counts once, and other last fours never count (rules.md B-26) | `tin_files.rs` (tests) `text_probing_for_a_tin_is_refused_alike_by_every_filing_tool`; unit tests `text_probing_for_a_tin_is_refused_alike_whether_or_not_it_holds_it` (the vault never opened for a probe), `one_or_two_candidates_are_compared_against_the_vault` |
| The connect text fits its budgets, its lead stands alone, it states the receipt rule enforced (exact threshold in the base currency, required and exempt accounts, reversals) and the parties rule in force, and the company name can't break it (control and format characters) | `handbook.rs` `instructions_budget`, `a_typical_company_gets_the_whole_connect_text`, `the_company_name_is_one_plain_line`; `receipt_policy.rs` `the_rule_told_is_the_rule_enforced`, `a_long_or_odd_account_list_stays_short_and_plain`; a new session is told the settings in force: `a_new_session_is_told_the_settings_in_force` |
| No text the agent is given (whole tool JSON, resources, connect text, handbook, every error template and refusal) points at files, git, GitHub, repo docs, flags or helper commands; descriptions fit 700 characters (listed exceptions, P5; and what git itself said of a failed backup, which `books.backup` passes on to the human as git said it, its paths hidden) | `tool_text_lint`, `tool_descriptions_fit_700_characters`; `ledger-core` `an_unknown_year_threshold_is_never_guessed` |
| Every tool is annotated (openWorldHint false on all but `books.back_up_now`), and the read-only hint is true in behaviour (journal, every data-dir file's contents but `ops/`, the viewer's side channel for `books.show`, which is not the books; the database's rows, the vault and the settings unchanged) | `annotations_complete`; `spec_mcp.rs` `read_only_tools_change_nothing` (checked on every call in `Mcp::tool`) |
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
  `handbook_tool_and_resources_over_stdio`). A `.jpg` or `.jpeg` W-9
  was taken (`every_w9_file_type_is_taken`); P2 slice 5 judges the W-9
  by its first bytes, not its name, so that mutant's code is gone (the
  test now checks each type by its bytes). The waker wakes: a request pending until another thread wakes it completes
  (`a_request_that_waits_is_woken_and_completes`). Three more were
  killed by tests of the import roots and their walk (a refused default
  root dropped, never fatal; a folder swapped for a link or a pipe never
  walked through; a read failure naming its kind, never the path): P2
  slice 5 removed `books.import_local_file` and the import roots, and
  those tests with them. `resolved` follows 40 links, as the kernel
  does, and no more
  (`resolved_follows_forty_links_and_no_more`). A file is saved only to
  a new file in a folder that exists, and a missing folder or a taken
  file is refused as such, never as a failed write
  (a CPA package test, and
  `the_w9_file_goes_to_the_vault_never_the_documents`; P2 slice 6 took
  the paths out of the tools, and the package test with them). The CPA
  package
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
  on `fd < 0`, now on `fd == -1`, as openat(2) specifies. (`inspect`,
  `read_checked`, the `openat` walk and that 25 MiB test were removed
  with the import roots in P2 slice 5.)
- **Equivalent (MCP, all 14 missed; the walk and its stubs were removed
  in P2 slice 5).** `^` for `|` in the open flags
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
| Settings need journal format 2: books this version made declared it as they were created, so a change journals only itself; a change to what is set journals nothing; books in format 1 refuse a change (nothing journaled) until their format upgrade, then make it after the one declaration; refused changes journal nothing | `a_change_needs_journal_format_2`, `refused_changes_journal_nothing` |
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

## P2 slices 3 to 8 (2026-10-19 to 2026-10-20)

From slice 3 on, mutation testing waits for P12 (D-33, owner 2026-10-20:
"put a pause on all the mutation testing … For now I want to focus on
the building"). Each slice was reviewed until a round came back clean and
passes `just check`; what P12 must still cover is listed at the end.

**Slice 3, opening and closing the books.** The books server on local
sockets (`ledger-mcp` `local.rs`, `admin.rs`, `opening.rs`), the host, its
leases and the agents' bridge (`ledger-host`). `ledger-cli/tests/
local_server.rs` (the server and its admin socket), `open_while_held.rs`
(the plan's named P2 tests under names of their own: last-holder close,
one host and server, the bridge opening closed books, cold start, restart
with writes across it, close during bursts of posts, SIGTERM and SIGHUP,
a clean host from a hostile bridge, respawn and backoff, no TCP, the app
scope, cancel, malformed input, a frozen host and server, books made
mid-session), `setup.rs`, `one_executable.rs`; unit tests in `ledger-host`,
`admin.rs`, `local.rs`, `opening.rs`; `ledger-store/tests/root.rs`
(adoption). Eight review rounds (13, 22, 9, 4, 4, 10, 3, 3 findings; the
last three found the core clean). The burst tests compare the posts in the
books with the posts answered.

**Slice 4, access.** Tokens, the door, scopes, the access log and the
owner passphrase (`ledger-store` `auth.rs`, `access.rs`, `chain.rs`,
`passphrase.rs`; `ledger-mcp` `scopes.rs`, `access.rs`): their unit tests,
`local_server.rs` (401/403, a session bound to its token, every call
recorded once with refusals, a damaged token log refusing every agent),
`open_while_held.rs` (the bridge picking up a token made mid-session),
`setup.rs` (the passphrase prompts, at a real terminal).

**One keyring item per token (2026-10-22).** On the live books a
reconnected bridge got no token from a keyring that couldn't be read,
was told to "make a token", and after `token create --replace` ran on a
token the keyring no longer gave while `download` read a revoked one.
The test keyring is now a folder per collection that a test can lock,
make unreachable, or plant a second item in (tokens.rs INVARIANT 4), and
the keeping and reading logic is shared with the OS keyring's.
`tokens.rs` unit tests (two items are said, never one of them given;
keeping removes every item first and reads back; a keyring that keeps
something else is refused; locked or unreachable is an error, never no
token), `bridge.rs` `a_keyring_that_cant_be_read_is_said_never_make_a_token`
and `each_new_session_reads_the_keyring_again` (against the scripted
server), `local.rs` `a_token_check_says_whether_the_text_is_the_live_one`,
`admin.rs` (`token_check` carries a hash, never a text), and
`open_while_held.rs`
`token_check_one_item_per_token_and_a_keyring_that_cant_be_read` (`token
check`'s every state end to end, a planted second item refused by
`check` and by the bridge and repaired by `--replace`, a bridge with an
unreachable keyring told so, a token the keyring can't keep revoked
again). The real Secret Service stays behind the ignored
`os_keyring_keeps_and_forgets`: tests never touch this computer's
keyring.

**Slice 5, files in and out.** Uploads by hash and size, exports handed
over by the server (`ledger-store` `uploads.rs`, `exports.rs`; `ledger-mcp`
`tools/files.rs`; `wanderbooks upload`/`download`): unit tests (expiry,
the grant replaced or lapsed while bytes arrive, a second send refused),
`local_server.rs`, `one_executable.rs`, `tin_files.rs` and the MCP specs
the import roots' removal changed. Four review rounds.

**Slice 6, the viewer.** Finding the books in the data root, settings from
the journal, presence through `ops/`, Save a copy and Open, the Export
menu, one builder for the CPA package (`ledger-package`): the viewer's
suites (`exports.rs`, `read_only.rs`, `show.rs`, `kept.rs`, `live.rs` and
the rest), `ledger-package/tests/package.rs`, `ledger-store` `saving.rs`
and `ops.rs`.

**Slice 7, backups and restore.** `ledger_store::backup`, `Root::restore`,
`ledger-mcp` `backup.rs` and `tools/backup.rs`, the loop in `local.rs`:
`ledger-cli/tests/backups.rs` (17: both targets after a write, the quiet
before a backup, close and catching up at the next open, admin only and
the head unmoved, a target that moved on, a failing target waiting while
the others carry on, a damaged record, restore beside and on a clean
install, of the previewed backup only, with its vault and key, refusing
damage, never overwriting, a failed replace, a restored copy leaving its
original's repository alone, a backup folder never made again, drills on
open and a failed drill flagged), `open_while_held.rs` (`books restore`
and its default id), `backup.rs`'s unit tests (20, among them links and
strangers refused before checkout, target and archive hooks never run, the
owner's hostile git configuration never spoiling a backup, repositories
with settings of their own or something beside them refused, fake gits
for the time limit and the process group), `tests/root.rs` (never moving
books something holds). Five review rounds (about 25, 15, 10, 7 and 3
findings, all fixed) and a sixth that only attacked the repository guard
(one low finding, fixed). The backup suite passed every run three at a
time.

**Slice 8, the viewer's menu, setup, no confinement (D-32).**
`ledger-gui/tests/owner_menu.rs` (each item through a fake owner, the
restore checked first, nothing of the books written, busy states;
screenshots looked at), `open_while_held.rs` (`books back-up`, the viewer
holding new books and closing the books, `connect` making anew a token
the keyring lost and saying when an agent refuses, the two lease
intents), `one_executable.rs` (the scaffold gone, and saying so),
`spec_kept.rs` (`isolation` is `same_user (advisory)`). Two review rounds.

**Slice 9, the cutover's put-aside.** `wanderbooks books put-aside ID
DATA-DIR [--vault DIR --vault-key FILE] [PATH...]` (root INVARIANT 8)
moves the old copies of adopted books whole into numbered folders of
their own in `legacy/`, a list of where each came from written first.
`ledger-store/tests/root.rs` (seven: a preview moves nothing; every byte
moved and the list written; refused while the books lack its journal, a
stored file, a vault record or the key, or when its head moved since the
preview; refused while held open, for a link, a path named twice, inside
the root or missing, and for a vault or key given as a plain path;
what adoption left behind and records the books sealed anew go aside
too; a registry rewritten is refused; a move cut short puts back what it
moved), `ledger-cli/tests/open_while_held.rs` (from the command line).
Three review rounds of data safety alone (12 findings, then 5, then 3
minor; all fixed but one: a vault record the old journal never named, a
crash between sealing and journaling, would be listed as the books'
newer copy; nothing is lost, the old vault going aside whole; hardening
for P7).

**Tests that mutation found, before the pause.** Pass 1 on slices 3 to 6
(each crate's own tests, the viewer left out): `ledger-store` 385 mutants
(44 missed), `ledger-package` 137 (38), `ledger-mcp` 497 (247),
`ledger-host` 666 (275), `ledger-cli` 47 (11). Pass 2 (the survivors again
with the end-to-end tests) left 38, 13, 90 and 11 in the first four; the
`ledger-host` pass was stopped. Then:
- `ledger-cli`'s 11 are all in the `#[cfg(not(unix))]` stubs
  (`owner_passphrase`, `run_files`, `run_local`, `run_server`): not
  compiled on Linux, so equivalent here;
- `ledger-mcp` (`access.rs`, `admin.rs`, `settings.rs`, `tools/files.rs`,
  `tools/kept.rs`): all 14 killed (each access record's request id, the
  admin socket's four-MiB line, `TokenText`'s and `CallError`'s text, no
  server meaning nothing sent, a book's own settings before it holds any,
  the caller's token in a printed command, a long name cut to 255 bytes,
  a settings file moving into the books). **One real bug:** the command
  line took an admin answer longer than four MiB, or cut off before its
  newline, whenever the bytes that came still parsed; it is refused now
  (`an_answer_is_one_line_of_at_most_four_mib`);
- `ledger-package`: tests for a dotfile's twin, receipts on the year's
  first and last days, a vendor's payments from the settings' accounts
  alone, the index's amount as written (`package.rs:482`'s debit side is
  not equivalent: amounts keep the scale they were written with), the
  TIN source's three cases. **Two real bugs**, found reading the naming:
  a twin's number went on the last dot of the whole path, so in a folder
  with a dot ("a.b") the second "notes" landed in another folder; and a
  name of dots alone ("..") could climb out of its folder when the zip is
  unpacked. Both fixed
  (`a_twins_number_goes_on_its_own_name_and_no_name_climbs_out_of_its_folder`);
- `ledger-store`: a private log stays 0700 (`chain.rs:204`'s two mutants
  are not equivalent); the two `flock` flag mutants in `ops.rs` (`|` for
  `^` on disjoint bits) are equivalent. The other 34 were triaged, not yet
  tested (masking keys one by one, a null TIN, token replacement across
  books, the chain's messages, export and upload expiry boundaries, the
  short-salt record, the dotfile and no-hard-link paths of `save_new`).

**For P12:** the whole diff from `168494b` (main before slice 3) to the
tip; `ledger-store`'s 34 triaged survivors; `ledger-mcp`'s `local.rs` (45)
and `server.rs` (31) survivors; `ledger-host`'s pass 2; slices 7 and 8 from
scratch (`ledger-store` `backup.rs` and `root.rs`, `ledger-core`
`settings.rs`, `ledger-mcp` `backup.rs`, `tools/backup.rs`, `local.rs`,
`admin.rs`, `ledger-host` `commands.rs`, `ledger-cli`). The survivor lists
are kept with the run's output, outside the repository.

## P3: odd cents (2026-10-20)

Defect 17 (rules.md B-13; D-10, D-38), built without its mutation run
(D-33); what P12 must cover is listed at the end. Every expected figure is
worked by hand in the test's comment.

**The named regression.** `the_audit_odd_cent_never_piles_up_across_closes`
(`ledger-core` `tests/odd_cent.rs`): an audit finding's −287.53 at 50/50,
anonymized, with a close to retained earnings between two members closes.
The run rule left A −287.54 / B −287.52; the rule since P3 gives −287.53
each (March: Dr 3000 143.76, Dr 3001 143.77). Its bookspec example is
`bookspec/examples/members-close.odd-cent-across-closes.toml`; it and the
other `members-close.*` examples are checked with every example by
`ledger-core` `tests/bookspec.rs` (`every_bookspec_example_holds`).

**Core** (`ledger-core` `tests/odd_cent.rs`): a true-up from inception
(`members_closes_true_up_from_inception`); a close that posts nothing
(`a_close_that_posts_nothing_carries_no_true_up`) and one that posts only a
sweep (`a_true_up_rides_on_a_distribution_sweep`, P3 review R-3); a reorder
(`reordering_members_keeps_counting_from_the_first_close`); a share change
and a return to the old shares (`a_share_change_starts_a_new_count`); a
member joining mid-year (`a_member_joining_mid_year_starts_a_new_count`,
R-8); a journaled start other than the derived one
(`a_journaled_start_other_than_the_derived_one_is_refused`); whole yen and
thousandths of a dinar (`a_yen_book_closes_to_members_in_whole_yen`,
`a_dinar_book_closes_to_members_in_thousandths`); the statement on a date
before and after its period closes
(`a_members_capital_on_a_date_does_not_change_when_its_period_closes`); a
pending true-up that shows only once posted
(`a_pending_true_up_shows_only_once_posted`, R-1); three members at 33.33%
/ 33.33% / 33.34% over twelve months, the statement checked against the
same entries never closed on every day of the year and the K-1 worksheet
at every month end (`three_members_at_a_third_each_over_twelve_months`).
Added by the P3 review: a gold book splits to the finest unit a line
carries, not cents (`a_gold_book_splits_to_the_finest_unit_a_line_carries`);
the K-1 worksheet shares contributed property in whole yen
(`a_yen_books_k1_shares_property_in_whole_yen`, and `share_property`'s unit
test in `k1.rs`); a reorder of the members in the settings moves no cent on
a day before the count's first close, nor in "other"
(`a_reorder_moves_no_cent_before_the_counts_first_close`). Each fails on
the code before the review's fix.
Properties (512 cases each): `members_capital_never_drifts_a_cent_from_their_share`
(a legacy prefix under the three earlier rules, then months closed to the
members in either order, to retained earnings or not at all; a close that
posts nothing changes no capital, R-3),
`each_members_capital_chains_and_holds_through_closes`,
`period_ends_rise_with_close_order` (R-10.6). In `statements.rs`:
`a_different_close_ends_a_legacy_run` (the run rule, still replayed) and
`a_different_close_no_longer_restarts_the_count`, the run rule's arm of
`a_run_of_closes_never_piles_up_rounding`, and February in
`a_partnership_closes_into_members_capital` (Bob 683.33 → 683.34: his share
of February's unclosed income is what its close will give, D-38). Unit
tests in `kernel.rs` (`apportion_cumulative_refuses_mismatched_or_out_of_range_inputs`,
`apportion_cumulative_trues_up_a_legacy_split`) and `periods.rs`
(`allocate_cumulative_*`: whole yen, a dinar, a loss, a 28-place share
refused, R-5). Bridges (`tests/kernel_bridge.rs`):
`apportion_cumulative_agrees`,
`allocate_cumulative_agrees_with_the_integer_model`.

**Store** (`ledger-store`): `members_closes_replay_under_the_rule_they_were_journaled_with`
(each rule; with a close to retained earnings between, the rule since P3
ends 0.01 / 0.01 and the run rule 0.02 / 0.00); `tests/member_closes.rs`:
`the_first_cumulative_close_declares_its_feature`,
`an_unknown_rounding_is_newer_not_corrupt` (R-9),
`a_count_that_starts_later_journals_its_start` (a share change: since_seq
2 journaled and replayed; added by the P3 review),
`a_close_journaled_with_another_start_refuses_to_open`,
`the_first_close_after_the_upgrade_trues_up_the_old_rules`, on the new
old-books fixture `x_member_closes` (written by main 00226a4c's Store API:
three untagged closes, three `"largest_remainder"`, `"cumulative"` broken by
a close to retained earnings; it opens to main's figures in
`books_an_earlier_version_wrote_open_to_its_figures`, R-2); `format.rs`
`an_older_reader_stops_at_the_members_cumulative_feature`.

**MCP and viewer**: `ledger-mcp` `close_period_shows_each_members_running_split`
(`tests/mcp_stdio.rs`: the preview's `members` is the confirmed one; after
a restart, `list_periods`' `close_rule` and the statement of members'
equity); `old_books_name_each_closes_rule_and_true_up_at_the_next_close`
(the old-books fixture `x_member_closes` through the server: every
`close_rule` an earlier rule gives, and December's true-up, the preview the
confirmed close); `ledger-gui` `a_members_page_shows_the_same_capital_before_and_after_a_close`
and `a_yen_members_page_splits_in_whole_yen` (what is owed to each member
and their property, in whole yen) (`tests/odd_cent.rs`).

**Proofs**: docs/verification.md (the three P3 laws).

**For P12** (nothing run here, D-33): `kernel::apportion_cumulative` (the
length and range guards; `checked_sub` for `checked_add`; the subtraction
skipped); `periods::allocate_cumulative` (the sign; `given` not negated for
a loss; `places` ignored; the exactness filter, zero products included);
`Ledger::members_regime` and `regime_where` (skip versus break at a close
to anything else; the order-free compare made ordered; the date filter
`<=` for `<`; `given` added to the wrong member; the count's order; the
next count's order as of a day before it begins);
`Regime::allocate` (the order); `Ledger::members_split` and
`close_period_to_steps` (the start check; the `posts` gate; the recorded
rule; the sweeps before the parts); `member_close_run` (it reads only
run-rule records); `member_close_split`; `reports::members_equity`
(`allocate(C)` replaced by `given`, R-1; `places`; "other"'s order);
`k1::share_property` and the viewer's `owed_to_member` (`places`); the store's
`close_to_from_journal` and `close_period_to` (the tag mapping; `since_seq`'s
parse; a tag that isn't text; the feature declaration);
`format::declared_knowing`; the MCP's `close_period` and `list_periods`
fields; `Currency::allocation_places`. P3's accept counts `ledger-core` and
`ledger-import` (R-6); the store and MCP mutants above are for the whole
pass.

## P3: the IIF and as-of (2026-10-20)

Defects 7 and 21 (plan P3; the owner's decisions D-36). Rules
`docs/spec/rules.md` B-33 (a package as of a day) and B-34 (the IIF). No
mutation run (D-33): what P12 must cover is at the end.

**Named regression tests**, each from its reproduction, on anonymized
fixtures:
- Defect 7, DOCNUM: `ledger-import` `tests/spec_import.rs`
  `iif_docnum_is_the_entry_id` (a draft never posted, three entries
  posted out of id and date order: DOCNUM 3, 2, 4 on the TRNS and every
  SPL; the file had written 1, 2, 3).
- Defect 7, the characters (D-36, plain ASCII):
  `iif_writes_plain_ascii_and_lists_every_change` (accented letters,
  Chinese, emoji, a fullwidth colon and fullwidth digits, a tab in a
  name, controls, a decomposed accent, private use: each written form
  exact, every change listed exactly), `iif_every_line_ends_cr_lf`,
  `iif_change_warnings_are_one_line_each`,
  `iif_keeps_a_tin_readable_to_the_package_scan` (an en-dash or
  no-break-space TIN in a memo stays a TIN to the package's TIN scan),
  `iif_distinct_accounts_never_share_a_name` (review R2: duplicate names,
  by case, siblings, a fullwidth colon against a hyphen, two private-use
  names; a name that is another's tagged form tagged in turn; the clash
  left when two codes are written alike refused),
  `iif_fits_quickbooks_desktop_limits` (Intuit's limits, found in the P3
  review: a level past 31 characters cut, two alike once cut tagged, a
  subaccount under its cut parent; an account number past 7 characters
  and two written alike left out of ACCNUM; a memo past 4,095 cut; every
  line of the warnings exact; a hand-built level past 31 refused),
  `iif_warnings_group_the_memos_by_what_is_written_differently` (the
  lead's call: 100 depreciation memos with an em dash make one line),
  `iif_warnings_never_hold_a_tin` (a TIN in a memo or an account name,
  in ASCII or fullwidth digits, never in a warning),
  `iif_field_transliteration_exact` and `iif_paths_and_transliteration`
  (the old expectations, "Caf? meals" among them, rewritten to the rule).
- Defect 21, the package's IIF: `ledger-package` `tests/package.rs`
  `as_of_package_iif_stops_at_as_of` (next year's rent was in the file;
  now the four entries of the year, by id, tied to the trial balance),
  `the_iif_through_as_of_at_its_edges` (an entry on as_of in, the day
  after out, a reversal after as_of leaving its original, as of a day
  before the first entry: the accounts and no transaction).
- Defect 21, every file: `every_file_in_an_as_of_package_stops_at_as_of`
  (review R4: books the same through as_of, one set with a 1099 payment,
  its receipt and an asset bought the day after, a reversal of the entry
  dated as_of, the members' contributions and an earlier desk sold, build
  the same package byte for byte, every file and which files there are,
  PDFs included, with and without TINs and receipts, with and without a
  fixed asset register, mid-year and at year end; a package days later
  sees the activity). Undoing any one of the as-of
  bounds (the IIF's entries, its FIXASSET typing, the 1099 worksheet's
  `through`, the fixed asset register's presence) fails it; checked by
  hand.
- D-36, the 1099 worksheet: `the_1099_worksheet_stops_at_as_of` (a
  payment on as_of counts, the day after doesn't; a vendor under the
  threshold so far is not yet reportable; a year-end package and the
  tools' worksheet unchanged).
- D-36, closes: `iif_leaves_out_period_closes` (two months closed: no
  close written, income and expenses equal the year's profit and loss,
  balances the trial balance without the closes).
- Plan P3's accept, a bookspec example per regression test: the as-of
  ones (`bookspec/examples/as-of-package-stops-at-as-of.toml`,
  `1099-worksheet-through-as-of.toml`) and the IIF's
  (`iif-quickbooks-journal-file.toml`, formats appendix), each expected
  value derived by hand. `ledger-core` `tests/bookspec.rs` loads every
  example, the formats appendix's too, and checks the as-of ones' trial
  balances, profits and losses and 1099 payments against the engine
  (`every_bookspec_example_holds`, the one loader of every example; the
  tests beside it refuse a query no crate answers, a format query outside
  the formats appendix, an unknown appendix, a payee not in the setup, a
  figure query leaving out an account, payee or entry line it covers, an
  as-of query given a period, `after` past the last event, members'
  shares not summing to 1 and a members close declared with another
  rounding, and show `after` asks the books after that event);
  `the_bookspec_iif_example_matches_the_engine`
  reads the IIF example and checks the engine gives exactly its file and
  changes.
- Review R18 and the README: `one_list_of_cash_accounts_per_package`,
  `the_readme_lists_what_the_iif_writes_differently` (the memos grouped:
  six depreciation memos and a lunch with an em dash, one line).

**Exhaustive and property tests.** `iif.rs`'s unit tests write every
one of the 1,112,064 Unicode scalar values (alone, after a letter, before
one) and check the field is printable ASCII without a double quote and a
fixed point, its changed runs empty exactly when it is unchanged, and a
level never empty, colon-free, trimmed and at most 31 characters (a
complete check over the finite alphabet; review R15); `with_tag`'s and
`changed_runs`' edges by hand; `the_transliteration_table_is_pinned`
samples deunicode 1.6.0, pinned exactly in `Cargo.toml` (R14).
Properties: `iif_round_trips_random_books` (memos mixing ASCII, Latin-1,
dashes and quotes, every kind of line break, controls, CJK, emoji,
combining marks, private use; DOCNUM, CR LF, the changes list exact),
`iif_names_never_clash_and_every_change_is_listed` (random charts from a
pool of clashing names, long names alike once cut, codes with colons,
past 7 characters, and one alike another once trimmed; every warning
line checked against the books and the file), `iif_ties_to_the_trial_balance_as_of` (2,000
cases: random books with reversals and closed months, random as_of; the
file's entries and balances against the trial balance without the
closes).

**End to end.** `ledger-mcp` `oracle_mcp.rs`:
`iif_declares_accounts_by_role_and_balances_every_transaction` (as_of,
DOCNUM the ids, the warnings exact: the fixture's line-broken memo and
the engine's own "Depreciation — Laptop — 2025-03" memos, the alias, a
range refused), `iif_without_as_of_is_as_of_today_and_ties_to_the_trial_balance`
(review R5), `cpa_package_holds_the_year_and_its_evidence` (the IIF from
the prior year's sale through as_of; a payment after as_of to the
contractor left out of the 1099 worksheet; the note's count);
`mcp_stdio.rs` (a Chinese account name over stdio: ASCII text, CR LF,
`warnings`); `spec_mcp.rs` (iif with as_of); `ledger-gui`
`tests/exports.rs` (the README's IIF line names the as_of).

**Names in every script, in the PDFs and the viewer.** Inter has no
Chinese, Japanese, Korean, Arabic, Hebrew, Thai or Devanagari glyphs:
every such character printed as Inter's empty box (glyph 0), and the
viewer drew boxes too. `ledger-import` `src/fonts.rs` now bundles the
fallbacks, each once: a subset of Noto Sans CJK SC
(`fonts/NotoSansCJK-LICENSE.txt` says how it was made) and Noto Sans
Arabic UI, Hebrew, Thai and Devanagari, unmodified
(`fonts/NotoSans-LICENSE.txt`: versions, upstream commits, SHA-256; 412,968
bytes for the four). Arabic is the UI form because Noto Sans Arabic 2.013
draws a letter's dots as marks of their own, which Typst writes as text
runs of their own: its PDFs' text read "شلام" for "سلام", with letters
doubled. No emoji font (about 10 MB): an emoji prints as an empty box in
the PDFs. Tests: `fonts.rs`
`every_template_names_every_fallback_in_order`; `pdf.rs`
`chinese_japanese_and_korean_print_as_written` and
`arabic_hebrew_thai_and_devanagari_print_as_written` (every glyph from
its script's font, none glyph 0); `spec_import.rs`
`pdf_keeps_chinese_japanese_and_korean_names` (the ToUnicode map) and
`pdf_prints_every_bundled_script_as_written` (the PDF read back with
hayro, the viewer's own reader: Arabic and Hebrew read right to left give
the words, the lam-alef ligature included; Thai, Devanagari and Chinese
left to right; every font embedded; failed with Noto Sans Arabic 2.013).
The viewer registers the same `FALLBACKS` after its own fonts in every
family (`ledger-gui` `theme.rs` `every_family_falls_back_to_the_noto_fonts`:
the very bytes, by pointer, and a real glyph for each script's letters in
every family) and `tests/scripts.rs`
`account_names_in_other_scripts_show_as_written` (the Accounts page paints
each name; with `SHOT_DIR` set, it saves `scripts.png`: looked at,
2026-10-20). egui shapes and joins each word, right to left for Arabic
and Hebrew, but doesn't reorder the words of a right-to-left phrase:
"مكتب الرياض" shows its two words left to right.

**TINs in any script (B-26).** `ledger_core::parties::has_full_tin`
missed "１２３－４５－６７８９": the fullwidth hyphen-minus, the ideographic
and thin spaces, the Hebrew maqaf and the other Unicode dashes weren't
separators, and a zero-width space or a right-to-left mark between digits
ended a run. Past ASCII every Zs space, every Dash character and the
compatibility forms of the dot, slash and underscore now separate, and
default-ignorable characters don't count (`docs/spec/sources/unicode-tin-text.md`,
Unicode 17.0); ASCII reads as before. `spec_parties.rs`
`has_full_tin_reads_every_script_and_separator` and the property
`has_full_tin_keeps_its_ascii_reading` (2,000 cases: on ASCII the same
answer as the old reading, kept in the test; elsewhere it finds at least
what the old one did).

**Second review (2026-10-20).** QuickBooks gives a subaccount its
parent's type (Intuit): `iif_a_subaccount_of_another_type_stands_at_the_top_level`
(a bank account under a header that is no bank, an asset's cost under
"Fixed assets": each written at the top level, keeping its type, listed,
its own subaccount with it, a name it then shares told apart by code);
`check_iif` now checks every subaccount's type is its parent's and every
parent is declared before it; `iif_declares_each_parent_before_its_subaccounts`.
The file names no customer or vendor (Intuit states no need for an IIF
journal; QuickBooks Desktop asks for one on A/R or A/P):
`the_readme_names_the_entries_on_receivables_and_payables`. A memo line
whose digits are masked says so (`iif_warnings_never_hold_a_tin`, which
now also holds a name and a memo that are a TIN only once written, digits
a tab apart: with `has_full_tin` reading every script, nothing else
tested the masks' second pass). `iif_fits_quickbooks_desktop_limits` now
holds a number of exactly 7 characters (kept) and two alike but for case
(left out): undoing either bound of the 7 had failed no test. Each was
checked by undoing it and seeing its test fail.

The GnuCash differential (`oracle/`) compares the engine's figures and
has no IIF and no package, so it has nothing to re-adjudicate here.

Each of the review's fixes was checked by undoing it by hand and seeing
its test fail (2026-10-20): the 31-character cut, the clash loop's
second round, the double quote, the TIN mask, the memo cut, each reason
a number is left out, the memo grouping, the tag's colon; and, for the
builder's, the as_of `<=`, the register's presence, the 1099 `through`,
the closes filter, the FIXASSET date filter and the package's cash list.

**For P12:** mutation over `ledger-import` `iif.rs` (each `breaks` arm,
the control, quote and `None` arms of `each_piece`, the deunicode
post-mapping, `segment`'s colon, trim, cut and "?" fallback, `tag`,
`with_tag`'s room, the clash loop's depth bound, rounds and tag,
`journal_to_iif`'s clash and length refusals, the number rules, the memo
cut, each `changes` comparison and its order, `warnings`' grouping,
order and masks, `shown`, DOCNUM, `EOL`), `pdf.rs` (the fallback font),
`ledger-package` `iif.rs` (`<=` in the as_of filter, the closes filter,
the FIXASSET date filter) and `package.rs` (the cash and financing
resolution, the register's presence, the 1099 `through`, the README's
IIF line, closes sentence, warnings block and 1099 wording), and
`ledger-core` `form1099.rs` (the `through` bound), `ledger-mcp`
`server.rs` (the iif report, range and as_of checks, `warnings`, the
package note), `ledger-gui` `menu.rs` (the note). From the second
review: `iif.rs` `iif_accounts`' type filter on parents and
`apart_from`, `parents_first`, the `AtTopLevel` change, the TIN note's
condition and `receivable_payable`; `fonts.rs` and `theme.rs`
`font_definitions`; the README's A/R and A/P sentence; `ledger-core`
`parties.rs` `tin_separator`'s arms and `tin_invisible`, and the gap that
doesn't count them.

## P3: cash flow (defect 1, 2026-10-20)

The statement of cash flows was rebuilt by the direct method (rules.md
B-14, D-35), and what a member or a lender pays for the company shown as
cash borrowed and spent (D-42, rule 11). Built, reviewed twice, `just
check` and `just prove`; no mutation run (D-33): what P12 must cover is at
the end.

- **Named regressions** (`crates/ledger-core/tests/spec_cash_flow.rs`;
  also bookspec examples, `bookspec/examples/cash-flow.*.toml`, checked by
  `tests/bookspec.rs` on every run):
  `defect1_reversed_contributions_foot_regression` (the minimal
  reproduction: a property contribution reversed, which broke footing by
  the contribution's amount), `defect1_two_member_llc_first_year_regression`
  and `defect1_two_member_llc_as_of_march_18_regression` (a replica of
  the 23 entries defect 1 was found in: the same shape, order, accounts
  and roles, Member A and B and generic vendors, every amount and date
  made up, so none is the real books'; to 2026-03-31, operating -596.24,
  investing 0.00, financing +2,216.40, the change in cash +1,620.16, and
  to 2026-03-18, operating -549.86, financing +2,549.86, worked by hand
  in the test's doc comment).
- **The rule, case by case** (same file; each oracle worked by hand from
  its paragraph in the test's doc comment): disposals at a gain, a loss
  and before any depreciation (`disposal_proceeds_all_investing`); a sale
  on an IOU (disclosed, flagged, with the fix); property contributed;
  what a member or lender pays or receives for the company (D-42, rule
  11): an expense through Due to Members, equipment a bank paid for, an
  expense paid into capital, a financing card, part cash and part a
  member's, a refund to a member, a customer paying a member, a payment
  made for the company and its reversal (in the period and alone, a
  prepaid and equipment too), the reimbursement, the uses' order (rule
  11's convention); and what stays noncash: property a member owned (with
  members configured and without), the seller's credit (flagged), interest
  and guaranteed payments charged to a creditor, a fee withheld on a
  contra-liability. From the second review: odd shapes (two cash accounts
  in one entry, one account on both sides, a reversal chain across the
  period start, a close: `odd_shapes_foot_and_net`) and taxes and
  government fees as other payments, (g); reversals in the
  period, alone, taking their root's sections and class, and reversals of
  reversals (review item 1); part cash (50-5); a fee withheld from a loan;
  interest with principal, paid and received; one entry per operating
  class (a)-(g); depreciation; cash spent on a depreciation expense account
  (D1-e); the reconciliation tying to operating; closes; the start date;
  transfers between cash accounts; debt converted to capital, a note
  received for equipment, a trade-in (review item 2); the bill split
  (D-35, through a bill restored as a later version would journal it),
  and a bill payment reversed in the period; a card purchase of
  equipment, on an operating and a financing card; each member's kinds;
  the five role refusals and `foot` directly. From the P3 review: interest
  classed apart from the rest of an entry's operating cash
  (`operating_cash_is_classed_kind_by_kind`) and kept operating in a
  disposal; a loss on disposal on a revenue-typed or an expense-typed
  account; an investment sold at a gain; a reversal three levels down;
  and both of rule 3's conventions pinned (the sections' order, the kinds'
  order).
- **Properties, 5,000 cases each** (`CF_CASES`, default 5,000, about 16 s):
  `prop_cash_flow_laws` over random books of 33 templates (every shape
  the rule names, rule 11's included) and reversals of any earlier entry,
  reversals included: the sections are the change in cash and the
  model's; each section is its lines and kinds; the reconciliation is
  operating; an entry's lines are its cash, what was handled for it
  netting to zero, each line's cash part with the entry's sign; every
  disclosure is its parts' positive sum or its exchange; an entry and its
  reversal in the period change nothing; periods split and add; closing
  every month changes nothing; scaling scales every figure.
  `prop_pure_entries_sections_are_movements`: for pure entries, each
  section is minus its accounts' movement over the entries that touch
  cash or that a member or lender settled whole for the company (the
  plan's wording, with rule 11 modelled apart from the engine; review
  item 7's definition of pure). The older properties in `spec_reports.rs`
  are kept with new oracles (financing is the cash part of equity and
  designated debt plus what the loan or capital paid or received for the
  company, by an independent model of rules 3 and 11).
- **Bridge:** `entry_flows_agree`, `apply_cash_and_exchanged_agree`,
  `constructive_agrees`.
- **Proofs:** Kani `entry_flows_sum_to_cash`,
  `entry_flows_pure_are_movements`, `entry_flows_are_bounded`,
  `no_cash_no_flow` (entries of up to three lines), the reversal's mirror
  (`entry_movements_of_a_reversal_mirror` and `flows_of_mirrors` at three
  lines, `entry_flows_of_a_reversal_mirror` end to end at two, since the
  P3 surface review), `take_is_odd_and_bounded`, `apply_cash_mirrors`,
  `apply_cash_spends_exactly`, `exchanged_is_bounded`,
  `constructive_nets_to_zero`, `constructive_mirrors`; Lean
  `cashflow_sections_sum_to_cash_change`, `entry_flows_sum`,
  `flows_reversal`, `constructive_nets`, `constructive_neg`,
  `constructive_sections_net`, `stmt_flows_sum`, `stmt_flows_reversal`,
  `operating_reconciliation_ties(_all)`, `cashflow_reversal_nets`,
  `Reachable.cashflow_foots`, `Reachable.cashflow_is_change_in_cash`
  (docs/verification.md, law 10).
- **Sensitivity (P3 review, by hand, no mutation run):** 18 deliberate
  breaks to `cashflow.rs` and `kernel.rs` (a dropped reversal
  flip, no netting, financing read as operating, no rule 4, no exchange
  disclosure, no interest class, no warnings, the bill's payments
  ignored, the label's depth parity, repeated codes allowed, depreciation
  disclosed, the sections' or kinds' order reversed, the customer party
  ignored, accumulated depreciation in the exchange, the interest
  exception dropped, rule 4 for investing accounts): 17 fail a test (4
  of them only since the review's tests); one is equivalent today
  (`mirrors` ignoring amounts: every reversal mirrors its original until
  P4's drafts).
- **Sensitivity, second round (after D-42, by hand, no mutation run):**
  17 breaks to rule 11 and the kernel's `constructive` (no lender pass, no
  member pass, no receipts for the company, a reversal not mirrored, a
  lender paying for no investing, capital paying for property, receipts
  for property, receivables not received, guaranteed payments or
  interest paid for, `on_behalf` not reported, the exchange on the net
  flow, owners' equity read as a lender, pairing that ignores
  `on_behalf`, the kernel's received sign, `max` for `min`, the uses'
  order reversed): 16 fail a test; one is equivalent today (pairing that
  ignores `on_behalf`: a mirrored reversal's real and constructive parts
  are each proved to mirror, Kani `entry_movements_of_a_reversal_mirror`,
  `flows_of_mirrors`, `entry_flows_of_a_reversal_mirror` and
  `constructive_mirrors`, so the check is defensive, for P4's drafts).
  The first round's 18 breaks, re-run on the reworked code (four
  adapted where the code moved), and one more for the taxes class: 18
  fail a test, `mirrors` ignoring amounts still the one equivalent.
- **End to end:** the printed statement's footing, schedules and rows as
  the engine's (`cash_flow_foots_and_reconciles`,
  `cash_flow_adjustments_appear_only_when_there_are_some`, ledger-import);
  the CPA package's statement with both schedules
  (`the_cash_flow_carries_both_schedules`); the MCP tool and its exports
  field for field against the core (`cash_flow_exports_equal_the_core_statement`),
  the CSV's warnings, the settings' roles and a member-paid expense over stdio
  (`cash_flow_reads_its_account_roles_from_the_books_settings`), refusals
  naming the setting (`cash_flow_refuses_contradictory_roles`), the
  settings' problems from the same check; the viewer's statement as the
  server's, a loan the setting leaves out operating in both, a refusal
  shown as a reason and no number, and the page painted
  (`the_cash_flow_page_is_the_servers_statement`,
  `a_refused_cash_flow_shows_why_and_no_number`,
  `the_cash_flow_page_paints_its_schedules`; with `SHOT_DIR`, a PNG); the
  statement's warnings as Needs attention items
  (`cash_flow_warnings_are_items`);
  every old-books fixture's statement computed or refused, never an
  engine error (`every_figure_over_old_books_is_computed_or_refused`).

**For P12** (D-33): `cargo mutants` on `ledger-core`'s `cashflow.rs`
(`cash_flow`, `analyse`, `pairs`, `label`, `class_of`, `reconcile`,
`warnings`, `bill_splits`, `role_problems`, `foot`, `on_behalf`) and
`kernel.rs`'s `take`, `apply_cash`, `entry_flows`, `constructive`,
`exchanged`; `ledger-import`'s
`report::cash_flow`; `ledger-mcp`'s `cash_flow_out`, `cash_flow_csv`,
`export_cash_flow` and `settings_problems`; zero unexplained survivors
(plan P3 accept). Likely equivalent, to adjudicate: `min` against `max`
in `take` where both bounds coincide, and the kind order's tie-breaks.

## P3: rename, defaults and tax categories (2026-10-20)

Defects 15, 16 and 8 (`plan/served.md` P3; rules.md B-30, B-31, B-32).
Every fixture is fictional (Fernhill Labs LLC, its members and vendors);
no mutation run (D-33): the targets are listed for P12 below.

**Named regressions**, from the brief's reproductions:
- `ledger-mcp` `tests/oracle_mcp.rs`: `show_refuses_start_after_end_defect_15`
  (1.1.1-1.1.2: refused, nothing reaches the viewer),
  `show_refuses_a_derived_inverted_range_defect_15` (1.1.3: a start after
  today with no end, the message saying the end was defaulted),
  `show_sends_the_dates_it_names_defect_16` (1.1.4: only an end covers its
  fiscal year to it; "as of" and "for" wording; the viewer gets the pair
  `shown` names), `show_refuses_options_its_target_ignores_defect_15`, and
  `show_writes_the_viewer_command_for_each_target` (resolved dates; a
  split never compared).
- `ledger-mcp` `tests/mcp_stdio.rs`: `one_default_period_everywhere_defect_16`
  (a July fiscal year, a first entry the year before: P&L, by class, cash
  flow, the exported tax summary, members' equity, general ledger and
  journal (csv, json: the period's entries only) and the CPA package all
  give 2025-07-01 through 2026-03-31 for that end; with no dates every one
  gives today's fiscal year to date, the clock read before and after, R14),
  `rename_account_end_to_end_defect_15` (the chart, a past period's P&L,
  the IIF's full names, verify, two journal lines, refusals write nothing,
  a restart keeps the name); `mcp_period_statements_over_stdio`'s old pin
  (no start = the first entry) now pins the fiscal year (02-01) that holds
  the end.
- `ledger-mcp` `tests/spec_tax_policy.rs`: `add_account_warns_without_tax_category_defect_8`
  (1.4.1), `require_tax_category_refuses_defect_8` (refused without,
  created with, the import refused whole, clearing refused, a bill's 6999
  created and named in its warnings and the digest),
  `a_new_account_named_like_a_sibling_is_refused`.
- `ledger-gui` `tests/view_model.rs`: `needs_attention_flags_every_uncategorized_pl_account_defect_8`
  (1.4.2-1.4.3: unused and last-year-only accounts listed, headers and
  categorized ones not; Info under warn, Todo under refuse), and
  `the_overview_numbers_are_the_books_numbers` now pins that the viewer's
  default report range, its register's default and the server's resolver
  agree on random books and fiscal years.
- `ledger-gui` `tests/show.rs`: `show_never_swaps_an_inverted_target_defect_15`,
  `show_report_with_only_end_uses_that_years_start_defect_16`,
  `show_all_dates_is_one_click_from_a_shown_register` (review R4).
- `ledger-store` `tests/store.rs`: `rename_account_changes_only_the_name_defect_15`
  (every figure, entry and earlier journal line, every projection row but
  `accounts.name`; refusals append nothing; reopen, rebuild and viewer),
  `a_journaled_rename_replays_past_the_command_rules` and
  `a_journaled_clear_replays_under_refuse` (B-28's pairs, review R5),
  `the_tax_category_policy_refuses_only_what_it_says`,
  `books_written_under_p3_keep_their_names` (fixture `old-books/p3_rename`,
  written once by `write_p3_rename_fixture`, `--ignored`; its figures are
  also in `books_an_earlier_version_wrote_open_to_its_figures`).
- `ledger-store` `tests/format.rs`: `p3_features_wait_for_the_upgrade_and_are_declared_once`,
  `an_unknown_feature_before_a_rename_is_newer_at_the_declaration`.

**Properties:** `prop_period_defaults_resolve` (`ledger-core`, 5,000
cases: every fiscal year start replay admits, every today and every mix of
given and omitted dates, the calendar's edges weighted);
`prop_rename_changes_no_figure` and `prop_tax_policy_refuses_exactly`
(`ledger-store`, `STORE_CASES`, 64 by default); the refusal law
(`refused_commands_never_reach_the_journal`) gains renames, accounts added
with a category, and the policy turned on and off; the core model
(`tests/model.rs`) gains `Cmd::Rename`, its model keeping a name per
account.

**Unit tests:** `calendar.rs` (each `PeriodDefaults` invariant and scenario:
fiscal years 01-01, 07-01, 12-01 and a lenient 07-15; the start day and
the day before; year 1 under a July year, clamped; 9999-12-31; a one-day
period; a start one day late; no month-day or chrono date panics);
`settings.rs` (`the_tax_category_policy_is_warn_or_refuse`); `ledger-config`
`codec.rs` (`the_tax_category_policy_round_trips`); `ledger-core`
`tests/spec_ledger.rs` (`rename_account_changes_only_the_name`,
`rename_account_refusals_leave_the_books_unchanged`,
`a_closed_period_or_a_role_never_blocks_a_rename`,
`a_journaled_rename_replays_past_the_command_rules`,
`a_new_account_named_like_a_sibling_or_holding_a_tin_is_refused`,
`a_rename_reason_is_bounded_text_without_a_tin`) and `tests/tax.rs`
(`needs_category`'s and the policy's truth tables, the catalog fitting
every account a tool requires on every return, tax-exempt income at 0%);
`ledger-mcp` `server.rs` `period_tools_describe_the_one_default` (every
report tool's description ends with its kind's sentence and each date
parameter is the shared constant; "all time", "beginning of time" and
"first entry" are gone from every tool and schema; the resources say
their period) and `show_reads_each_reports_dates_as_the_viewer_does`;
`handbook.rs` (the period rule rendered from the same constant);
`hostile_args.rs` `hostile_renames_ranges_and_categories_are_answered`.

Test fixtures that gave two accounts one name under one parent
(`ledger-core` `invariants.rs` and `spec_roles.rs`, the store's refusal
law) now name each for its code: the sibling rule is the commands', and
existing charts and journals are untouched (replay takes any name). The
viewer's `spec_gui.rs` small books give their one expense account a
category, so "empty books have nothing" still holds now that an unused
uncategorized expense account is listed.

**Adversarial review (2026-10-21).** Each finding has a test that failed
before its fix (checked by breaking the code and watching it fail):
- `ledger-core` `tests/tax.rs` `a_gain_on_business_property_is_figured_on_form_4797_on_every_return`,
  `ledger-import` `tests/spec_import.rs` `tax_summary_lists_form_4797_apart`,
  `ledger-mcp` `tests/spec_tax_policy.rs` `a_gain_on_business_property_is_exported_apart`:
  a `form_4797_gain_loss` amount was ordinary income on every return
  (Schedule C included, whose instructions send it to Form 4797; on a
  partnership net section 1231 gain is Schedule K line 10), its Tax column
  the book gain; it is now `TaxSummary::form_4797`, book only, in no
  total.
- `ledger-core` `tests/spec_ledger.rs` `a_new_name_is_one_trimmed_line_of_bounded_length`,
  `ledger-store` `tests/store.rs` `a_new_accounts_name_is_journaled_trimmed`,
  `hostile_args.rs` (the 100,000-character name now refused): a new
  account's name was journaled untrimmed while a rename's was trimmed,
  and names had no bound and could hold a line break or tab.
- `ledger-gui` `tests/view_model.rs` `all_dates_holds_an_entry_dated_after_today`:
  "Show all dates" and "All time" ended today, leaving out entries dated
  after it.
- `ledger-mcp` `tests/oracle_mcp.rs` `show_refuses_options_its_target_ignores_defect_15`
  (a split past 240 columns): `books.show` sent a split the viewer shows
  as one total.
- `ledger-mcp` `tests/mcp_stdio.rs` `rename_says_what_the_new_name_moves_defect_15`:
  the notes for the name-picked cash and card sets and conventions had no
  test of their own.
- `ledger-store` `tests/store.rs` `rename_account_changes_only_the_name_defect_15`
  now reads the projection's renamed name itself (its comparison blanked
  that column); `tests/format.rs` lost a check that recomputed the gate's
  answer itself.

**Landing on main (2026-10-21),** beside odd cents, the IIF and the
cash flow statement:
- `ledger-mcp` `tests/mcp_stdio.rs` `the_default_period_never_shortens_the_iif_or_the_package`:
  on a July fiscal year with entries before it, the general journal asked
  without dates is the fiscal year to date, while the IIF without dates
  holds every posted entry from the first through today under
  `general_journal`, `journal` and any capitals, refuses `start` and
  `end`, and the CPA package's IIF holds the entry before the package's
  fiscal year.
- `ledger-core` `tests/bookspec.rs`: the three examples are in the
  loader's one shape, and the loader gained what they need (bookspec
  `README.md`): `setup.today` and omitted dates resolved by B-30,
  `header` and `tax_category` on an account, a `bill` event, `[[changes]]`
  (a rename, the policy) made after an event's expectations, the queries
  `period`, `day`, `calendar_year`, `account_names` and
  `uncategorized_accounts`, and the commands `rename_account`,
  `add_account` and `set_tax_category`, tried on a copy and accepted or
  refused for a named reason. Each new key has a refusal test
  (`a_period_other_than_the_rules_is_refused`,
  `a_refusal_for_another_reason_is_refused`,
  `an_omitted_date_without_today_is_refused`,
  `account_names_leaving_an_account_out_is_refused`,
  `a_change_is_made_after_its_events_expectations`,
  `a_command_runs_under_the_policy_in_force`,
  `a_command_with_values_is_refused`).
- `ledger-import` `tests/spec_import.rs`: the IIF's tests
  (`iif_names_never_clash_and_every_change_is_listed`,
  `iif_distinct_accounts_never_share_a_name`,
  `iif_writes_plain_ascii_and_lists_every_change`,
  `iif_warnings_never_hold_a_tin`) make their siblings of one name, names
  with a tab and names holding a TIN by replay (`replay_add_account`), as
  books written before P3 hold them: B-31 refuses them in a new name, and
  the file must still keep them apart, write them on one line and mask
  them.
- `books.cash_flow`'s description is the direct method's, shortened to
  fit the 700-character cap with its dates sentence
  (`tool_descriptions_fit_700_characters`,
  `period_tools_describe_the_one_default`).

**For P12** (D-33; none run): `ledger-core` `calendar::{fiscal_year_start_unclamped,
fiscal_year_unclamped, fiscal_year_start_on, period_defaults}`,
`PeriodDefaults::{statement, statement_range, as_of, calendar_year}`,
`PeriodKind::describe`, `Company::fiscal_year_containing`,
`ChartOfAccounts::set_name`, `Ledger::{add_account (its new name checks),
add_control_account, check_account_name, rename_account,
check_rename_account, replay_rename_account, check_rename_reason,
check_tax_category_required, check_tax_category_kept}`,
`tax::{needs_category, account_needs_category, uncategorized_accounts,
on_form_4797}` and `tax_summary`'s Form 4797 section,
`Settings::require_tax_category`; `ledger-store` `Store::{ensure_feature,
ensure_features, journal_settings (its feature), add_account,
add_account_with_tax, journal_account_added, ensure_control_account,
rename_account, set_account_tax, journal_account_tax}`, the
`account.renamed` replay and projection arms, `format::setting_feature`,
`ops::report_columns`;
`ledger-package` `statement_range`, `as_of_date`, `pnl_statement`,
`bs_statement`, `cash_flow_statement`, `year_earnings`, `build_package`'s
fiscal year; `ledger-mcp` `show` (the option refusals, the resolve, the
`shown` wording), `show_report_dates`, `rename_account` (its notes),
`add_account`, `import_chart_csv` (both loops), `profit_and_loss_by_class`'s
range, `export_journal`'s period, `described`, `uncategorized_warning`,
`created_uncategorized`, the digest's `uncategorized_accounts`,
`handbook::period_rule`. The viewer is not mutated (owner, 2026-10-18):
`view_model.rs` and `show.rs` cover it.

## P3: parties (defects 14, 24, 11; 2026-10-20)

A party's part of an entry is signed as a line on its own account, with a
direction and its money and property; party totals take a basis; money is
cash, cards and the financing accounts; a credit to a liability a party is
owed itself never pays it; the directory's TIN last four comes from TIN or
W-9 facts (rules.md B-35, B-26). Built, reviewed and checked under D-33:
no mutation run here; what P12 must cover is at the end.

| What | Where | Oracle |
|---|---|---|
| The audit's table (F36, F37), row by row: resolved, direction, cash, in kind; reversals the exact opposite; bills and invoices signed; totals on both bases | `ledger-core/tests/spec_party_totals.rs` `defect14_shares_take_the_partys_side_and_reversals_flip` | B-35, by hand |
| Basis all is the members' equity statement, cash the K-1's cash contributions | `basis_all_is_the_members_equity_statement_and_cash_the_k1s`; property `member_totals_are_the_members_equity_statement` (members contributing and taking out money and property, alone and jointly, subject to a loan, through 2100 then converted, through their distributions accounts, depreciated property with money, declared to distributions payable and paid later, reversed later in the year; checked over every span from the year's start and, received less paid, over every span) | `reports::members_equity`, `k1::k1_detail` |
| A reversed member card payment nets to zero (brief §1.4) | `defect14_a_reversed_member_card_payment_nets_to_zero` | B-9 |
| Accruals through another liability count once (defect 24) | `defect24_an_accrual_through_another_liability_counts_once` | B-35 item 5 |
| A credit to a party's own liability never pays it: a card fee, a distribution owed, a seller's note, property subject to a loan | `defect24_a_credit_to_a_partys_own_liability_never_pays_it`; the documented limit, pinned: `a_distribution_owed_counts_twice_until_the_members_are_named_its_creditors` | B-35 item 6; Treas. Reg. §1.704-1(b)(2)(iv)(b)(2); Helvering v. Price |
| Review, 2026-10-21: a distribution declared to distributions payable counts once, when paid (it counted twice); a distribution of money and depreciated property splits by what moved (the money read as property); a gift card sold is received once (it was a "card": twice); a refund is received; tin_last4 never shows more than four digits | `defect24_a_distribution_declared_to_a_payable_counts_once_when_paid`, `a_distribution_of_money_and_depreciated_property_splits_by_what_moved`, `defect24_a_gift_card_sold_is_received_once`, `a_refund_is_received_from_the_vendor`, `defect11_a_partys_tin_last4_is_its_tin_facts_else_its_w9s` (SCENARIO 6) | B-35 items 3, 5; B-26 |
| A mixed contribution splits as the K-1 does, ties to the first member | `a_members_mixed_entry_splits_as_the_k1_does` | k1 INVARIANTS 3, 4 |
| Read from journaled facts: an archived member, a one-member company | `a_members_entry_keeps_its_reading_after_they_leave` | review R2 |
| A close is neither; a basis is cash or all; creditor_of's checks; tin_last4's five scenarios; the kernel's cases | `a_close_tagged_with_its_members_is_neither`, `a_basis_is_cash_or_all`, `a_creditor_is_named_only_on_a_liability_with_one_owner`, `defect11_a_partys_tin_last4_is_its_tin_facts_else_its_w9s`, `party_target_cases_by_hand` | specs |
| Properties: a reversal's target, parts and flow are the original's negated; parts sum to the target, each with its sign; a sole party's money part is the flow as before P3; tin_last4 is every source present | `a_reversal_is_the_exact_opposite`, `a_sole_partys_money_part_is_the_flow_as_before`, `tin_last4_is_every_source_present` (`PROPTEST_CASES`, default 256; 5,000 run clean) | B-35, B-9 |
| Decimal and i64 agree on party_target and reverse_target | `kernel_bridge.rs` `party_target_agrees` | the bridge |
| Kani: the direction is the sign, the target is odd, reverse_target an involution | `proofs.rs` `party_target_direction_is_its_sign`, `party_target_is_odd`, `reverse_target_is_an_involution` | the kernel spec |
| The journal: party.creditor_set one line each, under the feature the books declared when created; in books an older version wrote, refused (FormatUpgradePending, nothing journaled) until the format upgrade; refusals journal nothing; books without creditors never change, in either format; corrupt lines refused | `ledger-store/tests/party_creditors.rs` | the format gate |
| Over the MCP: get_entry's direction, cash, in_kind; party totals by basis, echoed, others refused; basis all equals members' equity; creditor_of round trips and a card fee isn't a payment; the post_entry preview's parts are what it posts | `ledger-mcp/tests/spec_party_totals.rs` | the tool docs |
| Defect 11 over the MCP: a member's TIN last four in the directory, a W-9's, none, after a purge | `ledger-mcp/tests/spec_vault.rs` `defect11_the_directory_shows_a_members_tin_last4` | B-26 |
| The viewer shows the tools' numbers, labelled cash basis, with the in-kind line; its what-for reads a reversal as `parties::flow` does | `ledger-gui/tests/parties.rs` `the_party_page_shows_the_basis_and_in_kind`, `a_reversed_member_card_payment_is_nothing_on_the_issuers_page` | `activity_with`, `flow` |
| bookspec | `bookspec/examples/p3-party-*.toml` | the same cases, by hand |

**For P12 (mutation targets, none run, D-33):** `kernel::party_target`
(each branch, the `!= 0` and `< 0` tests, the property and money clamps,
the neither-when-nothing-counts test), `kernel::reverse_target` and
`Direction::{flip, of}`; `shares::target` (the closing check, the reversal
arm and its parity, members_only, the equity filter and the retained
earnings exclusion, the property test, the own sums),
`shares::split` (the member-order tie-break, the cash shortcut, the zero
sign); `parties::{flow, root_original, money_accounts, card_accounts,
MoneyAccounts::for_parties}` (the negation of what_for, the walk's
guard, cards and financing as money, the gift-card exclusion); `activity_with` (the target before
the split, the skip of nothing-counts, the bucket by side and basis, the
overflow checks); `Activity::totals`; `Basis::parse`;
`Ledger::{set_party_creditor, check_set_party_creditor, creditors_of,
is_closing_entry}`; `Store::{set_party_creditor, ensure_feature}`; the
server's `parse_basis`, `party_activity`, `entry_parties_out` (direction,
cash, in_kind) and `update_party`'s creditor_of diff; the viewer's
`party_what_for` and `share_text`. Expected equivalent:
`Party::tin_last4`'s `or_else` order swapped (TIN facts and W-9 facts
never disagree, parties INVARIANT 2a, enforced on replay).

## P3: 1099s (defects 23, 12 and 18; 2026-10-20)

The 1099 decision kept as a decision (nec, misc, not_reportable with its
reason kind and words, undetermined), with who decided and the date it
takes effect (D-40); payment methods and the tag on each payment; one
report for the server, the package and the viewer (rules.md B-19). No
mutation run (D-33); the list P12 must cover is at the end.

| Law (rules.md B-19) | Tests |
|---|---|
| Undecided is not "not reportable" (defect 18) | `undecided_is_not_not_reportable` |
| "No 1099" is a decision with who and when; "none" refused (defect 23) | `a_vendor_marked_no_1099_is_not_undecided`, `the_cutovers_reasons_are_recorded_as_given`, `set_vendor_1099_says_what_a_decision_needs` (`ledger-mcp` `spec_1099.rs`) |
| A member's card is excluded, their check counts, unknown is undetermined (defect 12) | `a_members_personal_card_payment_is_not_a_1099_payment`, `payments_come_from_cash_and_financing_only` |
| Card-account payments are listed, not dropped | `card_account_payments_are_listed_not_dropped` |
| A bank payment counts unless tagged with a card (D-40) | `a_debit_card_payment_from_the_bank_is_excluded_when_tagged` |
| A tag describes one account (review R2) | `a_tag_naming_another_account_is_undetermined` |
| Money between the company's own sources pays no one (R1) | `paying_down_a_financing_liability_pays_no_vendor`, `paying_the_card_bill_pays_no_vendor` |
| Unknowns never settle the status (R3) | `a_negative_unknown_keeps_a_reportable_vendor_undetermined` |
| Money from a vendor for anything but a refund doesn't lower its 1099: a sale, its invoice paid, a disposal, a loan (R6) | `a_sale_to_a_contractor_does_not_lower_its_1099`, `money_from_a_vendor_that_is_no_refund_does_not_lower_its_1099`, unit `the_other_receipts_note_says_what_each_entry_did` (`ledger-package`) |
| Cash accounts named, dollars (R5, R16) | `the_report_needs_the_cash_accounts_and_dollars` |
| A shared payment splits across buckets to the cent (R12) | `a_shared_payment_splits_across_buckets_to_the_cent`, unit `a_vendor_share_splits_across_buckets_to_the_cent` |
| Dated decisions (D-40) | `a_status_change_mid_year_applies_by_payment_date`, `an_incorporation_on_december_30_is_a_new_vendor`, `an_llc_electing_corporate_taxation_keeps_its_vendor`, `a_dated_decision_records_a_change`, `a_decision_change_names_the_payments_it_reaches`, `an_undated_old_decision_is_in_force_from_the_beginning`, `two_status_changes_in_a_year_and_a_reversal_across_them`, unit `dated_decisions_take_effect_on_their_dates` |
| A change for a closed or due year warns (D-40, R15) | `a_change_reaching_a_filed_year_warns` (`spec_1099.rs`) |
| The bucket table, all 16 cases; splitting and netting exact | `payment_bucket_is_the_table_in_all_16_cases`, `buckets_net_money_moved_between_the_companys_sources`, Kani `payment_bucket_is_the_table`, `payment_split_is_exact`, `net_buckets_keeps_the_total`, bridge `payment_buckets_agree` |
| Properties | `no_instruments_no_financing_is_the_old_report` (the regression: with cash named and no methods or financing, the old figure exactly), `buckets_partition_each_payment`, `a_reversal_nets_every_bucket_to_zero`, `the_worksheet_ties_to_the_ledger` (paid + excluded + undetermined + not reportable + receipts for something else = every payment line in the year, dated decisions, sales and receipts included; the payment lists sum to their columns) |
| Payment methods: refusals, the reversal chain | `a_payment_method_is_refused_unless_sound`, `a_payment_method_tags_the_whole_reversal_chain`, unit tests in `instruments.rs` |
| W-9 flag kept apart from the decision (brief 1.5, R8) | `set_vendor_1099_keeps_the_w9_flag_when_omitted`, `set_w9_keeps_the_1099_decision_and_its_source`, `set_w9_asks_for_the_w9_flags_feature_before_its_first_line` (`spec_1099.rs`: refused whole in P2's books until their upgrade), `marking_a_w9_keeps_who_decided_the_1099` (`ledger-store` `tests/form1099.rs`) |
| Journal: lines written only under their features (refused, nothing journaled, in books an older version wrote until the format upgrade; never declared on the way); replay and projection exact; refusals and no-ops journal nothing; newer values newer; old books as they always meant | `ledger-store` `tests/form1099.rs` (`each_feature_waits_for_the_upgrade_and_is_never_declared_on_the_way`, `decisions_and_methods_replay_and_project`, `refusals_and_no_ops_journal_nothing`, `an_unknown_1099_value_is_newer_not_corrupt`, `books_written_before_decisions_read_as_they_always_meant`), unit tests in `ledger-store` `form1099.rs`, the op model (`refused_commands_never_reach_the_journal` with `SetVendor1099`, `SetW9Flag`, `AddInstrument`, `SetEntryInstrument`; an accepted command journals or changes nothing), `books_an_earlier_version_wrote_open_to_its_figures` (fixture `p2_vendor_1099`) |
| Records change no entry (verification 13a) | `decision_and_method_records_change_no_entry` |
| The quieter untagged note (R7) | `the_untagged_note_counts_only_money_out` (`ledger-package`) |
| One builder: the package CSV is the worksheet CSV for the package's accounts; README wording | `ledger-package` `tests/package.rs` (the zip's CSV equals `vendor_1099_csv` for the request's accounts, else the settings', through as_of: `payments_to_a_vendor_come_from_the_packages_accounts`, `one_list_of_cash_accounts_per_package`, `the_1099_worksheet_stops_at_as_of`; the README line pinned); over the MCP, the package's CSV is `books.vendor_1099`'s for the same `cash_accounts`, `financing_accounts` and `through` (`ledger-mcp` `oracle_mcp.rs` `cpa_package_holds_the_year_and_its_evidence`) |
| The viewer counts as the server does (brief 1.4) | `the_viewer_counts_the_1099_as_the_server_does` (`ledger-gui` `tests/parties.rs`) |
| End to end, restarting between steps | `payment_methods_round_trip_and_classify_payments` (`spec_1099.rs`), `mcp_vendor_1099_over_stdio`, `spec_mcp.rs` (every tool exercised and schema-checked, `books.instruments` read-only) |
| bookspec | `bookspec/examples/1099-*.toml` |

**For P12 (mutation targets, none run):** `ledger-core` `form1099.rs`
(`plan_decision`, `apply_decision`, `check_new_decision`'s decided-before
test, `in_force`/`in_force_before` (`<` against `<=`), `vendor_1099_report`'s
status branches (each `>=`, `<`, `max(.,0)`, `min(.,0)`), the listing
filter, the not-reportable and undetermined routing, the R6 test
(`< 0`, credit side, `refund_accounts` per account type), `Roles::new`
(which methods' accounts are financing, which a card's), `split_share` (the floor corrections,
the sign), `vendor_part`, the unresolved fallback, `decisions_during`,
`untagged_payments` (`> 0`, each skip, each debit kind), `decision_changes`);
`kernel.rs` (`payment_bucket`, `split_payment`, `net_buckets`);
`instruments.rs` and `Ledger::check_add_instrument` (each refusal),
`set_entry_instrument` (the chain), the reversal step's copy,
`set_vendor_w9_flag`; `ledger-store` `form1099.rs` (each decode check,
`newer_content`), `store.rs` (`ensure_feature`, `set_vendor_1099`'s
unchanged path, `set_vendor_w9_flag`, `add_instrument`,
`set_entry_instrument`'s no-op, the replay and projection arms,
`sqlite_reversal_chain`); `ledger-mcp` `tools/instruments.rs`
(`decision_in`'s refusals, `filed_year_warnings`' closed and due tests,
`resolve_instrument`, `add_instrument`'s warnings), `server.rs`'s
`set_vendor_1099` (the W-9-only path) and `set_w9` (the feature declared
first, only when the flag changes); `ledger-package` `vendor_1099.rs` (the CSV columns, every note's
guard, `may_file`, `missing_w9`'s suffix) and `package.rs` (tin_gaps, the
README line). The viewer has no mutation job (its musts are tested).

## P3: parties and 1099s landed together (2026-10-21)

The two pieces above, stacked on the IIF, cash flow and rename pieces, and
reconciled where they meet. Built under D-33 (no mutation run); what P12
must cover is at the end.

| What | Where | Oracle |
|---|---|---|
| One rule for what a card is: the recorded business cards' liabilities, else (none recorded) liabilities named "card", not "gift"; never a member's card, a member's check or a platform; a debit card on the bank is none; money follows it | `ledger-core` `tests/spec_party_totals.rs` `a_card_is_a_recorded_business_card_else_a_liability_named_card` | B-35 item 5, `parties::card_accounts` INVARIANTS 1-4 |
| Party totals and the 1099 worksheet never contradict: paid less received is the worksheet's paid + excluded_card + undetermined + not_reportable plus its other receipts, for a vendor named alone, whatever the methods, refunds and reversals | property `party_totals_are_the_1099s_buckets` (`ledger-core` `tests/form1099.rs`; `PROPTEST_CASES`, default 256; 2,000 run clean; dropping a bucket from the sum fails at once) | B-35 item 9, by hand |
| The same, one year by hand: a member's check, a member's card, a member's payment of unknown method, a company card charge and a company check to one contractor, then the member reimbursed | `bookspec/examples/1099-and-party-totals-for-a-members-payment.toml` | B-35 item 9; Treas. Reg. §1.6041-1(e)(1), (a)(1)(iv) |
| The CPA package's 1099 worksheet is `books.vendor_1099`'s CSV for the package's accounts (the request's, else the settings', R18) through its as_of, byte for byte; without `through` the files differ; a `through` outside the year and an unknown account are refused | `ledger-mcp` `oracle_mcp.rs` `cpa_package_holds_the_year_and_its_evidence`; `ledger-package` `tests/package.rs` `payments_to_a_vendor_come_from_the_packages_accounts`, `one_list_of_cash_accounts_per_package`, `the_1099_worksheet_stops_at_as_of` | B-33; mcp-interface.md |
| A feature is asked for before a command's first line: `books.update_party` with a field and the first creditor, in P2's books waiting for their format upgrade, is refused whole (nothing journaled, never the field without the creditor); once upgraded, as in books this version made, it journals the field, then the creditor, declaring nothing | `ledger-mcp` `tests/spec_party_totals.rs` `update_party_asks_for_the_creditor_feature_before_its_first_line` | B-27 |
| One bookspec loader for every example, the parties' and the 1099s' with them: `setup.parties`, `setup.members` (`distributions_account`), `setup.payees` (a party's vendor record), `setup.status_changes`, `setup.instruments`, events' `parties` and `instrument`, and the queries `vendor_1099`, `k1`, `party_part`, `party_totals`, `party_tin_last4`; refused: a party or payment method not in the setup, a reversal naming parties, party totals without a basis, a 1099 line leaving a figure out, a wrong direction or status | `ledger-core` `tests/bookspec.rs` (`every_bookspec_example_holds`, `a_party_not_in_the_setup_is_refused`, `a_party_parts_direction_is_checked`, `a_reversal_naming_parties_is_refused`, `party_totals_without_a_basis_are_refused`, `a_vendor_1099_line_leaving_a_figure_out_is_refused`, `a_vendor_1099_status_is_checked`, `a_payment_method_not_in_the_setup_is_refused`) | bookspec/README.md |

**For P12 (mutation targets, none run, D-33):** `parties::card_accounts`
(the recorded-cards branch, the gift and the other-instruments
exclusions); `ledger_package::Settings::with_accounts` (each list's
fallback); the server's `vendor_1099` (`through`'s year check, the
unknown-code check, the accounts passed) and `update_party`'s feature
declaration; the bookspec loader's party, payee, decision and payment
method setup.

## P3: depreciation and schedules (defects 22 and 9, 2026-10-20)

Every expected figure below was worked by hand from the rule (rules.md
B-15, B-16) and is written out in the test's doc comment; the fixtures are
made up (asset X: 12,000.00 over 12 months from 2026-01-01; a 1,200.00
policy from March 15). Each regression that is an accounting rule is also
a bookspec example (`bookspec/examples/`, P13, D-34).

| Law | Test |
|---|---|
| Through a day of the life's final month, depreciation is prorated by days, never plugged (repro A: December 483.87, accumulated 11,483.87) | `ledger-core` `spec_assets_payroll.rs` `defect22_depreciation_through_mid_final_month_is_prorated_not_plugged`; `spec_asset_tags.rs` `register_as_of_a_day_before_a_same_month_disposal_is_prorated` |
| A mid-month life's drift is spread over the final month's days (repro D: through Apr 13, 920.00; Apr 14, 930.00) | `defect22_final_month_drift_is_spread_by_days`; `spec_replayed_extremes.rs` `depreciation_reaches_chronos_last_month` (1,185.92 through Dec 10 of year 262142) |
| A disposed asset's schedule stops at its disposal day, held, and equals what the disposal took (repro B) | `defect22_a_disposed_assets_schedule_stops_at_its_disposal_day`, property `prop_disposed_schedule_equals_what_disposal_computed`; `ledger-mcp` `spec_assets.rs` `a_disposed_asset_lists_what_it_depreciated_through_its_disposal` |
| A disposal in the final month posts depreciation by days (repro C: 322.58, gain 322.58) | `ledger-store` `tests/assets.rs` `defect22_a_disposal_in_the_final_month_posts_depreciation_by_days` |
| A disposal inside a month already run takes back the days after it (R2: excess 500.00, book value 6,500.00) | `defect22_a_disposal_inside_a_posted_month_takes_back_the_extra_days` (store and `ledger-mcp` `spec_assets.rs`) |
| A month skipped is caught up in the month the run is made; a later run for it posts nothing (settled in design, P3) | core `a_skipped_month_is_caught_up_in_the_month_run`, store `a_skipped_month_is_caught_up_in_the_month_run`, MCP `a_skipped_month_is_caught_up_and_said` |
| A reversed run is un-posted and runs again (feature month_reposting); re-instating a run whose month ran again is refused; a disposal, and a run its disposal took as final, can't be reversed; a reversed run before a disposal leaves nothing behind (R3) | store `a_reversed_depreciation_month_can_be_run_again`, `a_disposal_entry_cannot_be_reversed`, `a_reversed_run_before_a_disposal_leaves_nothing_behind`; core `a_reversed_schedule_month_is_due_again` |
| What the books carry is read entry by entry, each in its own month: a run reversed after its month ended isn't run twice (its month posts nothing; the reversal's month catches it up), for depreciation; a schedule month reversed after it ended is left for a human, listed with why (P3 review) | store `a_run_reversed_after_its_month_is_caught_up_in_the_reversals_month`; GUI `a_run_reversed_after_its_month_leaves_the_reversals_month_due`; core `schedules.rs` `a_schedule_month_reversed_after_it_ended_is_left_for_a_human`; store `format.rs` `a_schedule_month_posted_again_waits_for_month_reposting_and_replays` |
| Re-instating a run (reversing its reversal) never carries the books past the schedule, nor past cost less salvage (P3 review) | store `reinstating_a_run_a_later_run_caught_up_is_refused` |
| What the books carry is each run and each reversal in its own month, alternately adding and taking away (P3 review) | core `spec_assets_payroll.rs` `depreciation_effects_read_each_entry_in_its_month` |
| A schedule month that rounds to nothing is never a posting: left out of a daily plan, never due in an equal-months one (P3 review; no test held it before) | core `schedules.rs` `a_month_that_rounds_to_nothing_is_never_a_posting` |
| A disposal is refused while a later month's run stands, or while an earlier run's reversal is dated after the disposal's month; once undone, the disposal month ends at 0.00 accumulated and its expense whole (P3 review) | store `a_disposal_before_a_later_months_run_is_refused_until_it_is_reversed`, `a_disposal_before_a_later_reversal_of_an_earlier_run_is_refused` |
| A disposed asset's figures in the viewer are what its disposal took (its excess or catch-up from the disposal entry) and none of its months is Due (P3 review) | GUI `a_disposed_assets_figures_are_what_its_disposal_took` |
| The final month is C + (D − C) × k / d, rounded half up, against arithmetic written in the test (P3 review) | property `prop_the_final_month_takes_the_remainder_by_its_days` (500 cases) |
| A month that rounds to nothing posts and journals nothing (R10) | store `run_depreciation_posts_nothing_for_a_month_that_rounds_to_nothing`, MCP `run_depreciation_skips_a_month_that_rounds_to_nothing` |
| Depreciation and daily schedules round to the currency's minor unit (R1: yen, dinars) | `jpy_depreciation_posts_whole_yen`, store `jpy_depreciation_runs_post_whole_yen`, `schedules.rs` `a_jpy_daily_schedule_posts_whole_yen`, `a_kwd_daily_schedule_rounds_to_the_fils`; `money.rs` `steps_are_the_currencys_minor_unit` |
| A new schedule spreads by the day (1,200.00 from March 15: 54.84, eleven 100.00, 45.16); deferred revenue is earned by the day (290.32, 333.34, 333.33, 43.01) | `schedules.rs` `defect9_a_new_schedule_spreads_by_the_day`, `defect9_deferred_revenue_daily_is_earned_to_the_cent`; `schedules.rs` unit `a_daily_plan_spreads_by_the_day`; `ledger-mcp` `mcp_stdio.rs` `mcp_daily_schedule_over_stdio` |
| Schedules from before P3 stay equal months, forever | `defect9_legacy_schedules_stay_monthly_even`, property `prop_monthly_even_plan_is_unchanged`; every old-books fixture (`books_an_earlier_version_wrote_open_to_its_figures`) |
| An equal-months schedule starts on the 1st (D-37); a daily one's last day is in the calendar | `an_equal_months_schedule_starts_on_the_first`, `spec_bounds.rs` `a_daily_schedule_whose_last_day_passes_9999_is_refused` |
| chrono's month-end clamping is pinned for daily plans (Jan 29, 30, 31; Feb 29) | `schedules.rs` unit `daily_plans_pin_month_end_clamping` |
| A new asset's cost and salvage are in the minor unit (replay takes finer ones, computed); a life ending 9999-12-31 is taken, a day later refused | `spec_bounds.rs` `an_asset_cost_finer_than_a_cent_is_refused`, `a_life_ending_on_9999_12_31_is_accepted`, `a_life_ending_after_it_is_refused` |
| A daily schedule needs feature schedule_convention (books an older version wrote wait for the upgrade); an older reader stops at its declaration; monthly_even needs none; an unknown convention is newer, never corrupt | `ledger-store` `tests/format.rs` `a_daily_schedule_waits_for_its_feature_and_an_older_reader_stops_there`, `a_monthly_even_schedule_declares_no_feature`, `an_unknown_schedule_convention_is_newer_not_corrupt`; `format.rs` unit `an_older_reader_stops_at_p3s_features` |
| Accumulated depreciation never falls, never exceeds cost − salvage, is 0 before service and the whole from the last day (sampled days: the first and final months, two between, a dozen month ends; R9) | property `prop_depreciation_is_monotone_and_bounded` (500 cases) |
| No posted month moves but a half-cent tie's cent (old Decimal arithmetic as the oracle) | property `prop_month_end_schedules_match_the_legacy_arithmetic_except_half_cent_ties` (500 cases) |
| A daily plan sums exactly, lands on month ends, each amount > 0; from the 1st, `months` postings within a cent of total/months | property `prop_daily_plan_sums_exactly_and_lands_on_month_ends` (2,000 cases) |
| Refused commands never reach the journal, now with daily and monthly_even schedules and zero-month runs | `ledger-store` `tests/store.rs` `refused_commands_never_reach_the_journal` |
| Every order of runs, reversals (any day, before an entry's own included), re-runs, re-instatements, disposals and closes on one asset keeps the books right: refusals journal nothing; at every month end before the disposal the expense is the accumulated depreciation and within the schedule; one run in effect per month; a disposal's month and the end at the schedule through its day, accumulated 0.00, the gain proceeds less book value; an active asset run month by month past its life takes exactly cost less salvage (P3 review round 2) | store `asset_orders.rs` `every_order_of_runs_reversals_disposals_and_closes_keeps_the_books_right` (exhaustive, three commands deep; four and five with `WB_ORDERS_DEPTH`, run clean 2026-10-21), `prop_any_sequence_keeps_the_books_right` (64 cases, up to 9 commands, any day) |
| The viewer reads an asset as the store does, whatever the runs, reversals and disposals: what the books carry, each finished month's status against a run of it, what is unposted (P3 review round 2) | GUI `prop_the_viewer_reads_an_asset_as_the_store_does` (48 cases), `the_viewer_counts_a_run_reinstated_after_its_rerun_was_reversed` |
| A run re-instated after its month was run again and that run reversed still counts: in the store, the disposal and the viewer (P3 review round 2; before, the projection's row named the reversed re-run) | store `a_run_reinstated_after_its_rerun_was_reversed_still_counts`; GUI `the_viewer_counts_a_run_reinstated_after_its_rerun_was_reversed` |
| A depreciation run's or schedule month's reversal can't be dated before the entry it reverses (P3 review round 2: a run of March reversed on Jan 31, then a disposal on Feb 15, ended February with a 1,000.00 debit in accumulated depreciation) | store `a_reversal_dated_before_the_entry_it_reverses_is_refused` |
| A disposal waits only until what the runs carry at its month's end is what they carry now, so a late change undone after its month closed lets it through (P3 review round 2; round 1 refused it for ever) | store `a_disposal_is_taken_once_a_late_change_is_undone_after_its_month_closed` |
| Depreciation still due when the disposal's month has a run is charged in the disposal entry, never as a second run of the month (P3 review round 2; no store test held it) | store `a_disposal_whose_month_has_run_charges_what_is_still_due_in_its_entry` |
| A command that needs a journal feature asks for it before its first journaled line: in books waiting for the format upgrade it is refused and journals nothing, and after the upgrade it is taken | store `format.rs` `a_write_waiting_for_the_upgrade_journals_nothing` |
| What a viewer's one pass says a run of each month posts is run_amount's for that month | property `prop_run_amounts_are_each_months_run_amount` (500 cases); Kani `catch_ups_is_every_suffixs_catch_up` |
| The kernel on i64 and i32 (Kani's) is the kernel on i128 and Decimal (the engine's) | `kernel_bridge.rs` `prorate_agrees_at_i64_and_i128`, `prorated_through_agrees_at_i32_and_i128`, `catch_up_and_true_up_agree` |
| The charge never falls and is continuous at month ends, over every 1-to-3-month span from every day of a common and a leap year | `kernel.rs` unit `the_charge_never_falls_and_is_continuous_over_real_calendars` (exhaustive) |
| Proofs | Kani `prorate_parts_sum_to_total`, `prorate_last_part_absorbs_the_remainder`, `a_months_charge_is_within_half_a_cent`, `a_months_charge_in_tenths_is_within_half_a_cent`, `the_last_months_charge_is_within_half_a_cent`, `the_last_months_charge_in_tenths_is_within_half_a_cent`, `prorated_through_refuses_malformed_input`, `catch_up_never_overtakes_the_schedule`, `catch_ups_is_every_suffixs_catch_up`, `true_up_reaches_the_target`; Lean `Prorate.lean` (`cum_mono`, `cum_le`, `cum_full`, `parts_telescope`, `prorate_total`, `prorate_nonneg`, `part_within_a_unit`, `roundUnit_near`, `roundUnit_scale`, `cum_near`, `part_near`, `chargeMid_eq_cum`, `chargeLast_eq_cum`, `chargeLast_start`, `chargeLast_mono`, `kernel_parts_total`, `kernel_parts_nonneg`; `docs/verification.md`) |

**Mutation targets for P12** (none run in P3, D-33): `kernel.rs`
`prorated_through` (the last/not-last branch, the rounding comparison
`r >= step - r` against `>`, the `k == days && last` shortcut, the cap at
the total, every refusal), `round_to_unit`, `prorate_by_days`,
`part_weight`, `MonthPart::is_valid`, the three `Units` impls, `catch_up`,
`catch_ups`, `true_up`; `calendar.rs` `month_parts`, `days_in_month`,
`next_month_start`; `money.rs` `Steps::of`, `Steps::decimal`; `assets.rs`
`schedule_months` (the stop date's `min` with the disposal day, the
inclusive `+ 1`, the partial-month break), `life_end`, `check_disposal`,
`run_amount`, `catch_up_room`, `depreciation_effects` (the chain's
alternating signs), `run_amounts` (its stop past the life), `cumulatives`;
`ledger.rs` `check_new_asset` (the minor-unit checks, the `pred()`
bound), `check_reversal` SCENARIO 5, `in_effect`, `reverse_entry_steps`'
schedule un-posting, `schedule_month_posted_before`,
`schedule_month_reversed_after`, `schedule_month_hold`; `schedules.rs`
`daily_plan`, `plan_for`, `validate_schedule` (the 1st rule, the daily
bound, the minor-unit plan check), `first_day`, `last_day`, `next`, `due`'s
zero months, `ScheduleConvention::parse`/`key`; `settings.rs` the new key
and `schedule_convention()`'s default; `store.rs` `run_depreciation`
(nothing due, the feature on a re-run), `live_runs`,
`Ledger::depreciation_runs` (was `depreciation_runs_ever`), `dispose_preview` (true-up split, `in_month`),
`check_nothing_after_disposal`, `disposal_lines`, `check_reverse_entry`
(the re-instatement room),
`register_schedule` (the feature only for daily), `ensure_feature`,
`post_schedule_month`'s feature, `run_schedules`' held months, the
`schedule.registered` replay arm's convention, the `asset.depreciation_posted`
projection upsert, `period_month`; `format.rs` `KNOWN_FEATURES`,
`declared_knowing`; `server.rs` `create_schedule`'s default resolution,
`schedule_out`'s new fields, `run_depreciation`'s skips, `list_assets`'
`schedule_error`, `dispose_asset`'s debits, `run_schedules`' held months;
`ledger-gui` `books.rs` `disposal_true_up`, `pages/assets.rs` `unposted`
and `schedule`'s disposed months. Equivalent mutants foreseen:
none (`r >= step - r` against `>` is killed by `half_a_unit_rounds_up`).

## P3: the format upgrade (2026-10-20)

D-39 and plan/served.md 3.9 (`format_upgrade_pending`): books an older
wanderbooks wrote wait for the owner's go-ahead before they move to this
version's journal format (rules.md B-27). Mutation testing waits for P12
(D-33); the targets are listed at the end.

- `ledger-store` `format.rs` unit tests: this version's format is the
  newest with every known feature; books wait exactly when behind (a
  synthetic version with features covers what main can't yet: features
  missing one at a time, all added at once, a format ahead never
  "behind"); a write needs what the books declared, and the refusal
  names the guard, says nothing changed, and that only the owner can
  agree; a reader that doesn't know a declared feature still refuses it
  as newer.
- `ledger-store` `tests/format_upgrade.rs`: new books declare the format
  as they are created; older books open without a byte changing, a change
  needing format 2 (settings, a convention, an open item) is refused
  journaling nothing while a change needing nothing new is made, the
  upgrade is one line with the whole format, again nothing, the figures
  unchanged; every old-books fixture (30) upgrades, verifies as it did and
  replays to its expected.json in the writer, reopened, and the viewer; a
  declared newer feature is still refused as newer. `tests/settings.rs`
  (`a_change_needs_journal_format_2`, the lazy declaration it replaces),
  `tests/root.rs` (an adoption's copy moves to this version's format; the
  source keeps its own), `tests/format.rs` (its books are format 1, as
  before the gate).
- `ledger-store` `tests/schema_files.rs` (each projection schema in a file
  of its own, the lead's decision of 2026-10-21): a P2-shaped books dir
  (`tests/fixtures/p2-books`, its `wanderbooks.db` at schema 12, written by
  `write_the_p2_fixture` while this code's schema was P2's) used as it is
  at its schema and left byte for byte, and whole, past it; a legacy file
  one schema older than this version's, on every version, left byte for
  byte and whole for its schema while this version's is built from the
  journal and kept up, the report and the viewer naming the one in use; a
  newer schema's file, under its own name or the legacy one, left alone,
  never refused; this version's own damaged (not a database, or holding
  another schema) kept aside and rebuilt, a damaged legacy one left; new
  books' file named for its schema. Checked by breaking it (the legacy
  file always taken, the schema check dropped): three tests fail. The
  store's other tests use `db_file()`, this version's file.
- `ledger-store` `backup.rs`: `the_archive_proves_to_hold_the_book_as_it_is_keys_and_all`
  (a changed or extra key, a changed vault record, books that moved on, a
  journal changed under the recorded head: each refused; the scratch
  folder always removed).
- `ledger-mcp` `local.rs`: `format_upgrade_waits_for_consent` (listed as
  waiting; no passphrase set, a wrong one or none refused before anything
  is done, no backup, no line, still served; with it, the backup is the
  archive's latest commit, holding the journal as it was and the key; one
  line, the whole format; on the backup record; again, nothing; new books
  wait for nothing, and asking for their upgrade makes not even a backup
  repository), `no_upgrade_without_a_backup` (a backup refused: nothing
  upgraded, still waiting, recorded) and
  `no_upgrade_unless_the_backup_just_made_is_proven` (an archive whose
  head names another branch takes the commit where no clone finds it:
  what is proven isn't what was made, nothing upgraded, both recorded;
  put right, the latest commit, which already holds the books, is proven
  and the upgrade made). Each checked by breaking the code it guards (the
  proof skipped, the made-is-proven check dropped, the early return for
  books that wait for nothing removed): the test fails, then passes once
  restored.
- `ledger-cli` `tests/format_upgrade.rs` (the plan's
  `format_upgrade_waits_for_consent`, from end to end through the one
  executable): a command not at a terminal says the books wait; `books
  list` and `doctor` say so; `books.start` names it and puts the handbook's
  format-upgrade section first; `books.set_setting` refused with the
  guard, journaling nothing, while adding an account works; no tool's name
  could consent, and no call moved the format; `books upgrade` refused off
  a terminal; the lease intent with a wrong passphrase refused; at a
  terminal an empty passphrase declines, and `books list` asks at each of
  two opens, while one whose output goes to a file only notes it on the
  terminal and asks nothing, its output its own; with the passphrase, the
  backup (journal as it was, the key)
  then one line, the agent's same session then sets the setting and
  verifies; asked again, nothing.
- `ledger-gui` `tests/format_upgrade.rs`: the dialog at open and at the
  next open, "Not now" leaving the standing notice (whose button, and the
  Books menu, ask again), the viewer writing nothing; nothing sent without
  a passphrase, the one typed handed to the owner as typed, a refusal said
  in the dialog, which stays, the right one closing it; books in this
  version's format and older books dirs never asked; an upgrade whose
  worker dies without an answer is said in the dialog, which can then be
  closed (never left at work, holding the window). Screenshots looked
  at (`SHOT_DIR=... cargo test -p ledger-gui --test format_upgrade
  screenshots`): the dialog over the overview, at work, refused, the
  notice, the menu item, done.
- Unit tests in `admin.rs` (the passphrase never in Debug; the request's
  shape), `ledger-host` `lease.rs` (`a_consent_never_shows_its_passphrase`:
  the lease intent's Debug, and the line's, name no passphrase), and
  `owner_menu.rs`'s fake owner takes the new call.

**For P12:** `ledger-store` `format.rs` (`current`, `upgrade_to`, `adds`,
`require_format`, `require_feature`), `store.rs` (`create_in_format`,
`ensure_format`, `ensure_feature`, `upgrade_format`), `root.rs`
(adoption's upgrade and its verify), `backup.rs` (`prove_archive`,
`held_files`); `ledger-mcp` `backup.rs` (`upgrade_format`,
`UpgradeFailed::parts`), `local.rs` (`upgrade_format`, the List's
`format_upgrade`, `waiting`), `server.rs` (`with_store_mut`),
`tools/kept.rs` (the first next step, `format_upgrade`), `admin.rs`;
`ledger-host` `host.rs` (`consent`, `consented`), `commands.rs`
(`consent_now`, `upgrade_format`, `offer_upgrades`, `offer_upgrade`,
`upgrade`, `book_line`, `upgraded_lines`, `upgrade_explained`,
`ASKED_UPGRADES`). None run.

## P3: asset basis and tax-basis capital (2026-10-20)

Defect 13 (assets: posted vs scheduled, the in-service and acquired dates,
the contributed property's bases) and D-41's K-1 worksheet on the tax
basis. Mutation runs wait for P12 (D-33); none was run.

- **Named regressions** (the brief's reproductions, anonymized):
  `list_assets_book_value_is_what_the_books_carry` and
  `a_reversed_run_is_not_posted` (`ledger-mcp/tests/spec_assets.rs`),
  `an_asset_bought_before_it_is_in_service_is_on_the_register_from_acquisition`,
  `personal_use_contribution_keeps_its_723_basis_apart_from_depreciable_basis`,
  `built_in_gain_is_fixed_at_contribution_not_book_cost`
  (`ledger-core/tests/spec_asset_basis.rs`, which replaces
  `spec_tax_basis.rs`). Each is also a bookspec example
  (`bookspec/examples/assets-*.toml`).
- **ledger-core:** `spec_asset_basis.rs` (every `AssetBasis::check`
  refusal and its replay pairs, the figure table for each rule, legacy
  records giving the figures they gave, the four-contributed-devices
  shape, the acquired date from the link and its refusals, the disposal
  floor, `accumulated_posted` and its edges: a run dated after `as_of`, a
  reversal dated after it, a reversal dated before its run, a reinstated
  run, disposal on and after `as_of`, `check_depreciation_run` and
  `record_depreciation_run`); `tax_capital.rs` (tax-basis capital per
  member with a cash contribution, a contribution of property, a
  distribution and a year of depreciation; next year's beginning; cash-only
  books, whose tax income is still the return's; property without a basis;
  property distributed; the roll-forward and reconciliation over spans;
  and, from the P3 review, four devices and a prepaid plan by hand (two
  worth less than basis, one more, one equal; the plan at book with its
  note; the gain property's disposal flagged with its built-in gain),
  property subject to a liability (negative tax property), and three
  members with unequal shares, one contributing nothing);
  `spec_asset_basis.rs` `a_duplicated_run_line_is_carried_once`; `properties.rs`
  `prop_basis_figures_laws` (5,000 cases); `kernel_bridge.rs`
  `basis_figures_agree`; Kani `basis_figures_laws` (`src/proofs.rs`, 83
  checks, its cover satisfied); `spec_asset_tags.rs` (the register's
  posted and scheduled figures, runs recorded as a store records them).
- **ledger-store:** `asset_tags.rs` (`asset.basis_set` journaled under the
  feature `asset_basis` (new books declare it as they are created),
  replayed, reloaded from the database and rebuilt, and in the viewer; a
  line whose figures disagree with today's formula still replays; an
  unknown rule or method is newer, a malformed figure corrupt; pre-P3
  `asset.tax_basis_set` lines give the figures they gave; P2's v12
  database left as it is while this version builds its v14 file, runs
  keyed by entry (`a_p2_database_is_left_and_this_versions_built_from_the_journal`);
  a v13 file as it shipped before the basis columns left too and v14
  built (`a_v13_file_built_before_the_basis_columns_is_left_and_rebuilt`;
  `schema_files.rs` `a_shipped_migration_never_changes`);
  v9 and v10 databases under this version's name rebuilt). The old books
  (`books_an_earlier_version_wrote_open_to_its_figures`) open to exactly
  their figures.
- **ledger-mcp:** `spec_assets.rs` (the aliases `purchase_date`,
  `in_service`, `tax_basis_rule`, `contributor_basis` and the old rule
  names, noted, and refused when they disagree; the acquired date from the
  acquisition entry; a value at contribution required; every refusal
  changing nothing; list_assets as of a date, disposed and not yet
  acquired; the register export's columns and totals).
- **ledger-package:** `package.rs`
  `the_k1_worksheet_reports_tax_basis_capital` (the CSV line by line).
- **bookspec:** the one loader (`ledger-core/tests/bookspec.rs`) runs the
  assets' examples in the shared shape: `setup.assets` with `acquired_by`
  and `basis`, a `[[changes]]` `correct_asset`, and the queries
  `register_row`, `register_totals`, `asset_basis`, `k1_tax_basis` and
  `k1_reconciliation` (keyed `"<member>.<line>"`, beside `k1`'s book
  figures); refused: a register row before the asset was acquired
  (`a_register_row_before_the_asset_was_acquired_is_refused`), an
  `acquired_by` naming an event with no entry
  (`an_asset_acquired_by_an_event_with_no_entry_is_refused`), a figure for
  a tax line the books can't know (`a_tax_line_the_books_cant_know_is_open`)
  and a member's reconciliation leaving a cause out
  (`a_members_reconciliation_shows_every_cause`).
- **ledger-import:** `asset_register.rs` (`pdf_shows_723_and_depreciable_basis`
  and the other PDF checks) and `spec_import.rs` (XLSX columns, totals,
  reconciliation and notes).
- **ledger-gui:** `tests/assets.rs` (`basis_figures_are_ledger_cores`,
  `journal_text_names_each_basis_figure`, the "Confirm its value" and
  "Choose a §704(c) method" items); `pages/assets.rs`
  (`an_assets_page_shows_its_own_book_value`: cost less posted, the
  schedule beside it); `pages/reports.rs`
  (`asset_register_report_carries_tax_basis`).

**For P12 (mutation targets):** `kernel::basis_figures` (min for max, the
sign of the gain, purchase to a contribution); `Ledger::accumulated_posted`
and `assets::run_chain_entries` (`<=` for `<` on the entry and reversal
dates, the disposed → 0 branch, the alternating sign); `live_depreciation_run`'s
reversal test; `check_depreciation_run`'s arms; the register filter
`acquired <= as_of` and `asset_acquired_date`'s min; `AssetBasis::check`,
`check_journaled` and `check_legacy_journaled` arms; the acquisition and
in-service date refusals; the disposal floor; the MCP `aliased` logic
(both given and differing); `asset_basis_from_journal` (newer vs corrupt);
the projection loader's two key sets; `tax_capital::share` and
`entry_property` (the mismatch branch, the unknown sum), the open-item
conditions (`in_service`, the disposal window), and the reconciliation's
signs.

## P3: the seams between the pieces (2026-10-21)

One year of fictional books for a two-member LLC, built through the real
`Store` (`ledger-package/tests/seams/books.rs`: cash and property
contributions, member-paid expenses through Due to Members and their
reimbursement, card purchases and payments, a daily prepaid, a 1099
contractor paid by check, card and a member's check whose status changes
mid-year, a refund, a loan, depreciation with a run reversed and caught
up, a disposal that credits back part of a month's run, a distribution
declared then paid, monthly closes with odd cents), checked on June 30,
September 15 and December 31: the same fact from two pieces agrees, or
rules.md says why not. Mutation runs wait for P12 (D-33).

- **Engine** (`ledger-package/tests/p3_seams.rs`):
  `the_members_figures_agree_on_every_date` (members' equity, the K-1
  worksheet, party totals on both bases, the cash flow's contributions,
  advances and distributions, B-35 items 4 and 8),
  `a_distribution_declared_then_paid_is_financing_when_paid`,
  `the_1099_worksheet_ties_to_party_totals_and_the_ledger` (B-35 item 9,
  B-19), `depreciation_is_one_figure_everywhere` (the P&L, the cash flow
  add-back, the tax-basis K-1's open item, the IIF, the register posted
  and scheduled, the balance sheet), `every_statement_foots_on_every_date`,
  `the_cpa_package_as_of_each_date_is_the_tools_figures` (B-33).
- **Viewer** (`ledger-gui/tests/p3_seams.rs`): its capital statements,
  party money, cash flow, register and 1099 worksheet, read from the
  books' folder and settings file as it opens them, are the engine's.
- **Tools** (`ledger-mcp/tests/p3_seams.rs`, the real binary over
  stdio): `books.cash_flow` line by line, `books.profit_and_loss`,
  `books.export` members_equity and iif, `books.list_assets`,
  `books.vendor_1099` through each date, `books.party` on both bases.
- **Found and fixed** (each test fails before its fix): a distribution
  declared to a payable and paid later read as cash paid to suppliers
  (B-14 rule 2; `a_distribution_declared_then_paid_is_a_distribution_paid`
  in `ledger-core/tests/spec_cash_flow.rs`, with the property
  `prop_cf_financing_is_equity_plus_designated_debt`'s model extended);
  a disposal's depreciation credit-back was in the cash flow's gain or
  loss, not its depreciation (B-14 rule 8;
  `a_disposals_depreciation_credit_back_is_depreciation`), and the
  tax-basis K-1's book depreciation left it out (B-36); `books.list_assets`
  as of a day said "disposed" for a later disposal (B-15 rule 8).

## P3: the surface review (2026-10-21)

The agent's surface, the docs and the whole proof run, on `main` with all
of P3 landed (first `2f999c6`, then rebased onto `2c5b5a7` with the
seams, asset basis and data safety reviews, and checked again). No
mutation run (D-33). What was run, in this
order, each alone (under `taskset -c 4-15`, four build jobs, on a laptop
other work kept at a load average near 30): `just check` (clean, before
and after the fixes); `just prove` as it stood (it ran past its
30-minute cap at four jobs, 52 of 53 harnesses verified: see
docs/verification.md, "Running it"); `just prove` and `just prove-slow`
after the split, until both passed; the viewer's screenshots on the demo
books; the privacy sweep below.

| What | Where | Oracle |
|---|---|---|
| Every tool the server has is documented in `docs/mcp-interface.md`, and every tool the doc documents exists (115; `books.access_log` had no entry) | `ledger-mcp` `server.rs` `every_tool_is_documented_and_every_documented_tool_exists` (with a check of its own reader) | the doc, read tool by tool against the code |
| The reversal's cash flow mirror at three lines, in two halves: a reversal's movements are its original's negated, and `kernel::flows_of` is odd; end to end at two lines | Kani `entry_movements_of_a_reversal_mirror`, `flows_of_mirrors`, `entry_flows_of_a_reversal_mirror` (`kernel::entry_flows` split into `entry_movements` and `flows_of`, behavior unchanged: every cash flow test, bridge and property passes) | docs/verification.md law 10; Lean `flows_reversal` |
| A closed-list value in a bookspec example (a 1099 form, a payment method's kind, a reason kind, a party totals basis) is spelled exactly as the README spells it; a K-1 query needs `cash_accounts` | `ledger-core` `tests/bookspec.rs` `a_form_in_another_case_is_refused`, `a_payment_method_kind_in_another_case_is_refused`, `a_reason_kind_in_another_case_is_refused`; `check_k1` | bookspec/README.md |
| Every Kani harness in `src/proofs.rs` runs in exactly one of `just prove` and `just prove-slow`, each run's count is checked, and a slow-list name that isn't a harness fails | `justfile` `_kani` | docs/verification.md, "Running it" |

**Docs corrected** (each against the code): `docs/mcp-interface.md`, about
60 fixes (parameter names: `search_entries`' `account_id`, `link_receipt`'s
`attachment_id`; outputs: `books.chart`, `books.digest`, `get_receipt`,
`run_recurring`, `label_asset`, `asset_schedule`, `dispose_asset`,
`import_bank_csv`; refusals: `reverse_entry`'s date rule,
`create_schedule`, `renumber_account`, `register_asset`, `correct_asset`,
`set_vendor_1099`, `set_entry_party`, `withdraw_attachment`; the
idempotent list (ten, not seven), the handbook's section list,
`books.start`'s next steps, the receipt policy's keys, the export reports
and formats, the CPA package's files, the close's targets, stale "out of
scope" text); tool and schema text in `server.rs` and `types.rs` where it
said the same wrong things; rules.md cross-references (eight wrong B-rule
numbers in code, tests and one example); bookspec's README (what the
loader takes, the statuses defined) and thirteen examples' statuses (an
example whose figures follow an owner decision is CHOICE and cites it);
docs/verification.md (laws numbered 1 to 31, no gaps; every harness listed
with its law, recipe and time); `README.md`'s tool count.

**Privacy sweep** (after the rebase onto `2c5b5a7`). `scrubber.originals_left`
over every tracked file (626, binaries decoded too): none in any text
file or fixture (the P2 served-books fixture's journal, database and
vault included); the six Inter font files match only a descending digit
run in their TrueType bytecode, a placeholder number the scrub's map also
holds, not a TIN from the books. The first company's people, places and bank, by name
(case-insensitive, LICENSE files aside): none. The eighteen figures from
its books, with and without thousands separators: none.

**The viewer** (the demo books, `cargo test -p ledger-gui --test screens`,
every page; PNGs kept outside the repo): the figures tie across pages.
Fixed: a report's notes were drawn on one line and cut (the fixed asset
register's explanation of its tax columns ended mid-word); they wrap over
the table's width now. The Accounts page listed each type's accounts in
the order they were added; it lists them by code, as the reports do.

**For P12 (mutation targets, none run):** `kernel::entry_movements` and
`kernel::flows_of` (the split of `entry_flows`; the same mutants as
before); the bookspec loader's exact-spelling filters and `check_k1`'s
`cash_accounts` check; `documented_tools` (its own sample checks it).
The earlier P3 sections' lists stand (one name corrected:
`Ledger::depreciation_runs`, which was `depreciation_runs_ever`).

## P3: projects (2026-10-21)

The owner's ask: "a Projects page that treats a class as a project ...
Projects are entities too", a company rule to link things to projects
(the convention `link-to-projects`), and the sidebar's links drawn under
the status box in a short window (rules.md B-37). Mutation runs wait for
P12 (D-33); none was run.

- **ledger-core:** `tests/spec_projects.rs`
  (`project_facts_default_for_a_bare_class`,
  `set_project_checks_every_field`,
  `set_project_renames_the_class_and_moves_no_figure`,
  `add_class_refuses_a_duplicate_name`,
  `replay_set_project_takes_what_the_text_rules_would_refuse`,
  `project_figures_are_its_tagged_lines`,
  `project_to_date_ignores_the_start_and_the_projects_dates`,
  `a_tagged_balance_sheet_line_counts_nothing`,
  `closing_entries_are_left_out_of_project_figures`,
  `a_reversal_nets_a_project_to_zero`,
  `unrecovered_cost_only_for_on_spec_and_never_negative`,
  `projects_of_party_lists_them_by_name`,
  `check_decision_refuses_an_unknown_or_repeated_project`,
  `differs_from_names_projects`,
  `names_compare_one_way_for_the_duplicate_check_and_the_lookup`,
  `old_books_duplicates_resolve_to_the_lowest_id_and_change_freely`,
  `an_unchanged_name_is_kept_exactly_and_not_checked_again`,
  `a_name_of_digits_only_is_refused`,
  `a_link_is_read_as_a_web_address_for_a_full_tin`); `properties.rs`
  `prop_project_nets_partition_the_pnl` (random tags on both lines of
  random entries, a random period: the projects plus the untagged lines
  are the P&L, and each project's months sum to its period) and
  `prop_project_entries_sum_to_the_net_to_date` (project_entries' own
  signs: the posted entries' project amounts are the net to date, with
  refunds, reimbursements, tagged cash lines and drafts beside them);
  Kani `unrecovered_cost_laws` (`src/proofs.rs`, docs/verification.md law
  32, run by `just prove`: never above
  max(expenses, 0) when revenue isn't negative; the earlier "never above
  expenses" fails it, checked).
- **ledger-store:** `tests/projects.rs`
  (`add_project_journals_class_added_then_project_set`,
  `a_name_only_project_journals_class_added_alone`,
  `project_writes_wait_for_the_upgrade_and_journal_nothing`,
  `update_with_nothing_new_journals_nothing`,
  `project_set_replays_verifies_and_reloads`,
  `an_unknown_status_or_kind_is_newer_a_malformed_record_corrupt`,
  `a_decision_without_projects_writes_the_bytes_it_always_did`,
  `attachments_link_to_a_project_and_unlink`,
  `old_books_open_to_their_figures_with_projects_known` (each fixture's
  report snapshot and verify against its expected.json),
  `format_2_books_without_the_feature_wait_for_it_alone` (the live
  books after install: format 2, every earlier feature, not `projects`),
  `old_books_with_a_duplicate_name_change_either_project`,
  `a_fact_only_change_never_renames_an_untrimmed_old_name`, and the proptest
  `prop_project_set_round_trips`); `tests/format.rs`
  `p3_features_wait_for_the_upgrade_and_are_declared_once` gains
  `projects` and a `project.set`.
- **ledger-mcp:** `tests/spec_projects.rs`
  (`add_project_then_project_returns_its_facts_and_figures`,
  `a_name_only_project_journals_only_class_added`,
  `update_project_checks_every_field_before_writing`,
  `update_project_with_nothing_new_journals_nothing`,
  `project_figures_match_profit_and_loss_by_class`,
  `project_to_date_counts_lines_before_the_period`,
  `on_spec_project_shows_unrecovered_cost_and_others_null`,
  `project_lists_entries_documents_decisions_and_party`,
  `link_attachment_to_a_project_by_name_and_id`,
  `file_document_with_a_bad_project_files_nothing`,
  `record_decision_names_projects_and_decisions_filter_by_project`,
  `party_lists_its_projects`, `start_lists_active_and_paused_projects`,
  `show_project_sends_its_target`,
  `project_writes_wait_for_the_format_upgrade`,
  `add_line_notes_a_finished_project`,
  `record_payout_notes_a_finished_project`,
  `a_name_resolves_as_the_duplicate_check_compares`,
  `old_books_duplicates_take_fact_only_changes`,
  `a_digit_name_that_shadows_an_id_says_what_to_pass`); `hostile_args.rs`
  `a_project_over_the_whole_calendar_answers_quickly` and
  `books.projects` among the read-only calls; `spec_vault.rs`'s idempotent
  list gains `books.update_project`; the unit tables (`annotations_complete`,
  scopes' table) list 119 tools, `period_tools_describe_the_one_default`
  checks the period tools' wording, and `mcp_stdio.rs`'s `tools/list`
  shows 116; `every_tool_is_documented_and_every_documented_tool_exists`
  holds with the four new tools (on the rebase onto the surface review it
  caught `books.list_classes` written mid-line and a `books.set_line_class`
  the doc named, which is no tool). `ledger-store` `tests/format.rs`
  `the_replay_fuzzer_writes_every_event_type` caught `project.set`
  missing from `fuzz_store_replay`'s types; it and the project fields are
  in them now.
- **ledger-gui:** `tests/projects.rs`
  (`projects_page_lists_every_project_alphabetically`,
  `project_figures_are_ledger_cores`, `unrecovered_cost_shows_only_on_spec`,
  `a_project_row_opens_the_project`, `an_entry_row_opens_the_entry`,
  `a_document_opens_the_document`, `the_party_link_opens_the_party`,
  `profit_by_project_bar_opens_the_project`,
  `party_page_lists_its_projects`, `entry_line_project_opens_the_project`,
  `journal_describes_project_set`, `show_project_target_opens_its_page`,
  `a_long_link_stays_in_its_card` (200 characters, 1440 and 900 wide),
  `a_long_name_keeps_the_page_in_the_window` (with spaces and without),
  `a_long_party_name_keeps_to_its_card`,
  `a_month_bar_opens_nothing_and_margin_keeps_one_place`,
  `an_entry_without_receipts_says_nothing_of_them`; with `SHOT_DIR` set
  the edge pages are saved as PNGs); `charts.rs`
  `a_crowded_chart_keeps_its_years` and
  `a_view_only_chart_shows_figures_and_takes_no_click`;
  `old_books.rs` `old_books_classes_read_as_projects` and every project
  page over the old books; `read_only.rs` renders the Projects list and
  every project page over read-only books (what the brief called
  `visiting_project_pages_changes_nothing`); `breadcrumbs.rs` a project's
  crumbs. The sidebar: `edges.rs`
  `sidebar_links_never_draw_under_the_status_box` (1440x500, scrolled to
  the end: Journal ends above the status box),
  `rail_links_never_draw_under_its_footer` (900x420) and
  `a_normal_window_shows_every_link_unscrolled` (1440x920); both short
  ones fail on the code before the fix (checked by putting it back);
  `a_very_short_window_keeps_one_link` (1440x220, 900x200: a link stays
  whole above the footer, which gives way off the bottom). The
  sidebar links, painted table rows and bars now carry accessible names,
  which the tests find them by. Shots before and after:
  `~/.cache/wanderbooks/shots-p3-projects/` (`before-*`, `after-*`,
  `final-*`, and the project pages light, dark, 1100 and 900 wide).
- **bookspec:** `setup.projects`, a line's `project`, and the query
  `project`; examples `projects-net-is-its-tagged-lines` and
  `projects-on-spec-costs-are-expenses`; refused:
  `a_project_named_twice_is_refused`,
  `a_line_naming_an_unknown_project_is_refused`,
  `a_project_query_leaving_a_value_out_is_refused`.

**For P12 (mutation targets):** `check_set_project`'s arms (ended_on with
active/paused, `<` vs `<=` on the dates, the duplicate name's
self-exception, case folding) and `ProjectFacts::check` (the link limit,
whitespace, duplicates); `kernel::unrecovered_cost` (`max`, the sign);
`project_figures`' class filter, its 240-month window and the
closing-entry exclusion; `project_entries`' normal-side sign and the
balance-sheet zero; `project_set_of` (newer vs corrupt);
`Store::add_project`'s "facts beyond the default" test (one line vs two);
`set_project`'s unchanged short-cut; `subject_feature`; `check_link_to`'s
feature check; the decision `projects` omit-when-empty;
`late_tag_note`'s `is_over`; `remember_footer`'s threshold.

## P3: the project snapshot (2026-10-22)

The owner's ask: a "mini financial snapshot" of one project on its page,
overhead left out, so it shows at a glance whether the project is in the
black or the red, with charts (rules.md B-37 item 7: contribution before
overhead, revenue less every traced cost, a segment margin in Garrison's
sense; a presentation choice, not GAAP). Every figure is ledger-core's:
`ProjectToDate::{net, margin_pct, any_tagged, revenue_by_account,
expenses_by_account, revenue_lines, expenses_lines}`,
`ProjectFigures::{months_net_to_date, period_margin_pct, period_lines}`
and `contribution_margin_pct`, which `books.project` and
`books.projects` now return too (`margin_pct`, `to_date.by_account`,
`by_month[].net_to_date`). No mutation run (P12, D-33; none on the GUI);
each test below was checked by breaking what it guards and watching it
fail.

- **ledger-core:** `tests/spec_projects.rs`
  `contribution_margin_is_one_place_and_none_without_revenue` (64.0,
  100.0, -150.0, half to even at 1.25 and 1.35, none at zero or negative
  revenue, none, not a panic, on overflow),
  `months_net_to_date_runs_from_before_the_months` (opens on last year's
  deposit; a reimbursement above its cost leaves the expense below zero;
  no entries: every month zero, no margin),
  `a_reversal_nets_a_project_to_zero` (no account left, two lines
  counted: tagged, breaking even; a period before both counts none) and
  `closing_entries_are_left_out_of_project_figures` (a close's lines
  aren't counted); `properties.rs`
  `prop_project_contribution_is_the_net_to_date` (random tags, sides,
  refunds and reimbursements over two expense accounts and a random
  period: the accounts sum to the totals, the contribution is revenue
  less expenses, each month's net to date is `project_to_date` through
  its end and the last is the net to date, the counted lines are the
  tagged revenue and expense lines to date and in the period, a margin
  exists exactly when revenue is above zero and has the contribution's
  sign, and `project_figures`' to-date is `project_to_date`'s).
  `project_figures` takes the net before its first month from to date
  less the months (one scan fewer); the proptest pins it.
- **ledger-mcp:** `tests/spec_projects.rs`
  `project_snapshot_margin_by_account_and_running_net` (the period's and
  to date's margin_pct, to_date.by_account, the 12 months' net_to_date
  opening on the December deposit and ending on to_date.net, a loss's
  negative margin, null without revenue, a project with nothing tagged
  still carrying `to_date.by_account: []`, and books.projects carrying
  the margin but not the accounts). `books.project`'s description stays
  under 700 characters (`tool_descriptions_fit_700_characters`).
- **viewer and server on one book:** ledger-cli
  `tests/projects_parity.rs`
  `a_projects_snapshot_is_the_same_in_the_viewer_and_from_the_server`
  (`wanderbooks dev-stdio`'s `books.project` against the viewer's
  `page_figures` and `Snap` on the same books: to date's revenue,
  expenses, net, margin_pct, unrecovered cost and by_account, every
  month's figures and net_to_date over the chart's span, and the fiscal
  year's; with a refund, a reversal and overhead; gui builds only).
- **ledger-gui:** `tests/projects.rs`
  `a_profitable_projects_snapshot_is_ledger_cores` (and the Contribution
  tile's hover note is `CONTRIBUTION_NOTE`),
  `a_loss_making_project_is_in_the_red` (costs largest first: the
  higher-coded account is the larger),
  `an_on_spec_projects_snapshot_has_its_unrecovered_cost` (each tile's
  own value, read under its label, equal to `Snap::to_date` of
  `project_to_date`, the pill, the margin shown only with revenue,
  overhead never counted),
  `no_revenue_means_no_margin`,
  `a_project_with_no_entries_has_nothing_tagged_yet`,
  `a_cost_repaid_exactly_breaks_even` (no account left: break-even, not
  "nothing tagged", on the page and the list),
  `a_reimbursement_above_its_cost_leaves_costs_below_zero`,
  `a_project_started_this_month_charts_one_month`,
  `the_chart_runs_from_the_start_or_first_entry_and_caps` (the earlier of
  start and first entry, a start after today, 24 months at most),
  `the_running_net_is_each_months_net_to_date` (sweeping the pointer over
  the bars and over the net's own strip shows each month's
  `months_net_to_date`, in order),
  `the_fiscal_year_switch_shows_the_default_period` (B-30's omitted dates,
  as `books.project`, with fiscal-year costs unlike to date's; the pill
  stays to date; switching writes nothing to the journal),
  `a_fiscal_year_with_nothing_says_so`,
  `the_list_cues_in_the_black_and_in_the_red` (each net painted in its
  cue's color, never the names', still alphabetical), and the proptest
  `prop_snapshot_contribution_is_the_net_to_date` (reversals too: a
  reversed tagged line is tagged and breaks even); the earlier tests take
  the new labels (Revenue, Contribution, Contribution margin). With
  `SHOT_DIR` set each snapshot case is saved as a PNG; run them one at a
  time then (`SHOT_DIR=... cargo test -p ledger-gui --test projects --
  --test-threads 1`: rendering from tests run in parallel crashed with
  SIGSEGV, 2026-10-22; without `SHOT_DIR` nothing renders and they run
  side by side). `screens.rs` gains `project-templates`
  (the demo's profitable project). Shots:
  `~/.cache/wanderbooks/shots-p3-snapshot/` (`light/`, `dark/`, `w900/`,
  `tests/`).
- **Every test honors TMPDIR.** `.cargo/config.toml` forced `[env]
  TMPDIR = "/tmp"` for every cargo-run test (for `just mutants*`' speed),
  so a TMPDIR set to keep tests off a small RAM /tmp was overridden;
  it is gone. Every test makes its dirs with `std::env::temp_dir()` or
  `tempfile` (both TMPDIR), and the binaries tests spawn with a cleared
  environment write no temp files. A socket under a long TMPDIR is longer
  than a socket address holds: every test connects through
  `ledger_store::root::SocketPath`, as the servers bind (`raw_admin` in
  `local_server.rs` didn't, and failed with TMPDIR under
  `~/.cache/wanderbooks/p3-snapshot-tmp`). ledger-gui's `exports.rs`
  downloads folder, shared by its tests, is removed as the binary exits
  (`atexit`; `wb-*`, so `sweep-test-tmp` catches a killed run). Checked
  by running `just check` with TMPDIR under `~/.cache/wanderbooks` and
  finding no new test dir in /tmp. Mutation runs (`just mutants*`) now
  put test books under that TMPDIR too, on disk: slower per mutant than
  /tmp was; none is running (P12 paused).

**For P12 (mutation targets):** `contribution_margin_pct`'s `<=` and
rounding strategy; `project_figures`' opening net and running sum;
`profit_and_loss_counted`'s counts; `Standing::of`'s order.

## Invoices and bills take a project (2026-10-22)

rules.md B-37 item 8: each invoice and bill line takes an optional project
(`books.create_invoice` / `books.create_bill` `class`); the entry's revenue
or expense side is one line per project (`arap::document_parts`), the
receivable or payable and the payments untagged; journaled as the line's
`class_id` behind the new feature `document_line_projects` (B-27).

- **ledger-core:** `tests/spec_document_projects.rs`
  (`an_invoices_revenue_is_one_line_per_project_and_the_receivable_none`,
  `a_bills_cost_is_one_line_per_project_and_the_payable_none`,
  `an_untagged_document_posts_as_before`,
  `a_line_naming_an_unknown_project_is_refused_whole`,
  `a_finished_project_still_takes_an_invoice`,
  `tags_move_only_the_project_figures` (twin books, tagged and not: the
  P&L, the 1099 worksheet and a party's activity identical; the projects
  with the untagged lines partition the P&L),
  `document_parts_refuse_past_exact_arithmetic`,
  `reversing_a_tagged_document_takes_its_projects_to_zero` (a partial
  payment moves no project figure; the reversal carries each line's
  project, so every project nets to zero), and the proptest
  `prop_document_parts_partition_the_total`); bookspec
  `projects-invoice-and-bill-lines-count-by-project` (the loader gains an
  `invoice` event and a document's `lines` with `project`; loader tests
  `a_document_line_naming_an_unknown_project_is_refused`,
  `a_document_with_an_amount_and_lines_is_refused`,
  `an_invoice_lines_project_is_what_the_project_counts`).
- **ledger-store:** `tests/document_projects.rs`
  (`a_tagged_invoice_journals_each_projects_line_and_replays`: the exact
  lines, then verify, reopen and the viewer's snapshot;
  `a_tagged_bill_tags_its_expense_lines_never_the_payable`;
  `an_untagged_document_journals_as_before`: no `class_id` anywhere;
  `a_document_naming_a_project_waits_for_the_feature`: books with
  `projects` but not this feature refuse, journaling nothing, not even
  the control accounts; `old_invoices_and_bills_replay_unchanged`: an
  earlier version's journal opens, verifies, writes nothing;
  `a_document_lines_class_id_that_isnt_an_id_is_corrupt`;
  `a_reversed_tagged_invoice_replays_with_its_projects_at_zero`: one
  `entry.reversed`, its replay tagged as live); `format.rs`
  `document_line_projects_is_its_own_feature`.
- **ledger-mcp:** `tests/spec_projects.rs`
  (`invoice_and_bill_lines_count_toward_their_projects`: by name, id and
  any capitals; `books.get_entry`'s line `project` and `class_id`;
  payments untagged;
  `books.project`, `profit_and_loss_by_class`, `vendor_1099` and
  `books.party` totals; `a_document_naming_an_unknown_project_is_refused_whole`,
  `a_document_for_a_finished_project_says_so`,
  `an_invoice_for_an_on_spec_project_asks_about_an_agreement` (an
  invoice's note, a bill's none),
  `books_without_the_feature_refuse_a_tagged_document` with the
  handbook's text both ways, by `books.handbook` and as the
  `books://handbook/projects` resource); `handbook.rs`
  `document_projects_follow_the_books_feature` (no sentence lets a shared
  cost be split on a document; the allocation convention only when
  `books.conventions` has one).
- **ledger-gui:** `tests/projects.rs`
  (`an_invoice_and_a_bill_show_each_lines_project`: each document line's
  project a link that opens it, untagged lines bare;
  `the_demos_client_work_counts_every_client_invoice`);
  `pages::entry::tests::a_tagged_line_keeps_its_amounts_in_their_column`
  (in a narrow pane a tagged line's name is cut to leave its tag room:
  looking at the demo's invoice page at 1,100 points found the tag pushing
  the credit column past the pane, a fault every tagged entry had);
  `tests/screens.rs` shoots an invoice's and a bill's page.
- **ledger-cli:** `tests/projects_parity.rs`
  (`invoiced_and_billed_project_figures_are_the_same_in_the_viewer_and_from_the_server`:
  made through the server's tools, read by both, the month lists the same
  length and not empty); `tests/format_upgrade.rs`
  `books_p2_served_cut_over_whole` refuses a tagged invoice and bill
  before consent and takes them after.
- **fuzz:** `fuzz_store_replay` can now declare features (this one
  among them), so a document line's `class_id` reaches replay under a
  declared format.

**Broken to confirm (each restored):** the core's revenue/expense line
left untagged (core 4 and store 3 tests fail); `document_parts` one part
per line (4 core, the proptest among them); the store's feature check
dropped (store gate test, MCP `books_without_the_feature_...`, CLI
`books_p2_served_cut_over_whole`); `document_line_json` without
`class_id` (3 store); `parse_document_lines` ignoring it (4 store, the
corrupt-id test among them); the `entry.line_class_set` not journaled (3
store, by verify); the MCP ignoring a line's `class` (4 MCP); no late-tag
notes (1 MCP); the handbook ungated (the handbook unit test and 1 MCP);
the handbook's rule without the owner's decision of D-43 (one project or
none a line, no percentage, a shared cost by the allocation convention:
the handbook unit test);
`books.get_entry` without `project` (2 MCP, the CLI parity test); the
viewer's document lines dropped, or their project not a link (the GUI
tests); no room kept back for the tag (the narrow-pane test); the demo's invoices untagged (the demo test); the bookspec loader
dropping a line's project (`every_bookspec_example_holds`). After
review: the projects handbook's old "split a shared cost across lines"
sentence restored, or the convention made unconditional (the handbook
unit test); no on-spec note, or one on a bill too (the on-spec MCP test);
the handbook resource served as if the feature were declared
(`books_without_the_feature_...`); `books.get_entry`'s `class_id`
dropped (`invoice_and_bill_lines_...`); the server's months cut short
(the parity test, which `zip` alone let pass); a reversal's line tags
dropped (the core and store reversal tests); the bookspec's Harbor made
on spec again (`every_bookspec_example_holds`). A separate
up-front unknown-class check in the core was removed: equivalent to
`set_line_class`'s own refusal inside `atomically`.

**For P12 (mutation targets):** `document_parts` (the first-appearance
order, the `contains` test); `add_document_parts`' tag index;
`ensure_document_lines`' `any`; `document_line_json`'s `if let`;
`parse_document_lines`' arms (null, absent, out of `u32`);
`document_notes`' dedup and its `invoice` flag; `on_spec_invoice_note`'s
kind test; `document_projects_rule`'s branch; the viewer's
`document_of` (invoice before bill).

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
| 7 | A member's ending on one equity statement may differ by a cent from the next statement's beginning | CHANGED 2026-10-13; CHANGED P3 | A capital account is one running balance (Treas. Reg. §1.704-1(b)(2)(iv)(b); the Item L quote this row once gave was misread, P3 review R-4). Undistributed earnings now split cumulatively as of each date, so each member's capital chains exactly (`each_members_capital_chains_across_statements`; it fails on the old code); since P3 the split is what the count's next close would give, so a date's capital never moves when its month is closed (D-38; `each_members_capital_chains_and_holds_through_closes`); rules.md B-13 |
| 8 | Cash flow nets one line per account; investing comes only from registered assets | CHANGED 2026-10-13 | ASC 230-10-45-7: gross receipts and payments "more relevant" than net; 45-8 limits netting to quick-turnover items. Financing and investing are now gross; loans made and investments bought are investing (230-10-20, 45-13) via `[books] investing_accounts`. Owner-advanced expenses: PwC 6.9.3.1 constructive treatment via `[books] financing_accounts` (rules.md B-14) |
| 8a | Cash flow by the indirect method; member-paid expenses as constructive receipts | CHANGED 2026-10-20 | The direct method with the reconciliation as a separate schedule (ASC 230-10-45-25, 45-30); noncash investing and financing activities disclosed, only the cash portion reported (50-3, 50-5). Member-paid expenses stay constructive receipts and disbursements, now gross and on each payment's date, and the same for a lender's payments and an expense paid into capital: a choice (D-42, after a day as noncash under D-35), since ASC 230 doesn't address payments made on the entity's behalf and the firm guides present them so (PwC 6.9.3.1, DART 7.2, BDO 3.4, EY 4.6.2; KPMG Q8.3.10 also accepts noncash for property a lender pays for). rules.md B-14 rule 11, P3 defect 1 |
| 9 | An asset's life ends at purchase + life months, that day not held; day-count proration | CONVENTION | Exactly N months of service; day proration and the mid-month convention are both accepted book policies (ASC 360-10-35-4, read 2026-10-14: any systematic and rational allocation) |
| 10 | A payroll entry has one line per account total | CONFIRMED | The per-employee detail the IRS requires ("Amounts and dates of all wage ... payments", Employment Tax Recordkeeping) is kept in the run; a summary entry is standard |
| 11 | A period with exactly the dates of an existing one is allowed | DESIGN | Nothing governs duplicate periods; harmless (the second close locks only). Refusing exact duplicates is an option |
| 12 | Each close into members' capital rounds on its own (ties to the earlier member) | CHANGED 2026-10-13; CHANGED P3 | Twelve monthly 50/50 closes could leave one member several cents ahead. A run of members closes split cumulatively from 2026-10-13 (Lean `runPart_telescopes`; property `each_member_stays_within_a_cent_of_their_share` fails on the per-close rule), but any other close restarted the run. Since P3 a count of members closes runs from the company's first close (D-10), skipping closes to anything else and ignoring the members' order, so no member is ever a cent from their share (`the_audit_odd_cent_never_piles_up_across_closes` fails on the run rule; Kani `cumulative_close_never_drifts_a_unit_at_*`, Lean `allocW_cumulative_drift`); journaled `"cumulative_since"`, older closes replay as booked (rules.md B-13) |

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
