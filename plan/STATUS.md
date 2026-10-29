# STATUS — 2026-10-28: nine units landed, five built on paused branches; P11a starts

## Now (2026-10-29): the wave has landed; S4.5 next, P11a step 0 beside it

**Where things stand.** `main` is `bfd453b6`. The five branches paused
on 2026-10-27 are merged, in the 8.0.2 order, each with `just check`
green after it: S4.4 (`d329bd6f`, 2,883 tests), S5 (`3a3f03d8`, 2,893),
S6 (`be580598`, 2,912), S7 (`7ebeeed2`, 2,923; `just prove` green: 52
Kani harnesses, 95 Lean theorems) and P6 (`9351f89d` with its fixes;
the batch's one full gate on `bfd453b6`: 170 suites, 2,944 tests
passed, 0 failed, 7 ignored, 77 minutes). Landed since the hand-off of 2026-10-26: fourteen
units. Not started: S4.5, P4, P5, P8, P10, P11a, P11b, P12, P13 steps
4-6. The live books are unchanged since the P3 cutover and the
`document_standing` upgrade of 2026-10-24; `documents_v2`, `drafts_v2`
and `attachment_text` wait for the one upgrade after P4 (D-57).

**What the merges found.** The P4b branches merged as the dry runs
predicted (the `documents.rs` add/add, the `types.rs` appends, laws 35
to 37, schema 17 then 18). P6 was different: its checkpoint's gate had
never run to the end, and the four units merged before it did not know
its dedup rule (the same bytes filed twice are one record, linked to
each subject). Three things followed, all fixed on `main` and recorded
in P6's As-built block: S7's write-off attached its evidence after
journaling the entry, so a notice whose bytes were already on file was
refused by the one-receipt-one-entry rule after the money had moved
(the check now runs before the journal line); the replay fuzzer and the
journeys count lagged the new event; and five tests the interrupted
gate never reached still encoded the invariant P6 replaced, or counts
from before extraction records existed (`bfd453b6`). No golden with a
figure in it moved; the one golden re-blessed is the sidebar's shape
hash, whose attention, event and file counts changed with the fixture.

**CI.** `check` and `two-user` are green on every push since
`fb83af59`; the two-user job needed the fixture copies owned by the
server account (`79fb7ff2`), and its first green run is P9a's Done-when
1. The weekly mutation shards are gone from `scheduled.yml` (D-59). The
weekly `fuzz_settings_replay` crash is fixed on branch `ci-fixes`
(`76285434`: the fuzzer used the renamed database path) with a
regression test for the one `fuzz_store_replay` finding, a writer and a
viewer disagreeing on an unknown convention kind, which does not
reproduce locally (the hand-written case passes; ten local minutes
found nothing); the job now keeps its crash input as an artifact for
the next run.

**Answered 2026-10-29 (the question tool):** Q-9, the writer's dependency
on the formats crate: the reader moves into a small crate of its own
after S4.5 (D-61, unit U0d, half a day). And D-60, the owner's rule for
how much checking a merge gets: a compile and the unit's own tests per
merge; the exhaustive gates once per batch.

**Next, in order:** (1) S4.5, the last P4b slice (`ci-fixes` merged,
`95a54d69`); (2) U0d, the reader crate, beside it; (3) P11a step 0 (the macOS and Windows keyring stores, the Windows data
root and pipes, a cross-platform CI build) and the packaging trial,
staffed now; (4) P4 (adding `confirmed` to the settlement tools too),
then the owner's one format upgrade (D-57), P5, P8, P10; P13 steps 4-6
beside; P11b after P11a; P12 last.

**Housekeeping done 2026-10-29:** the five unit worktrees and their
`target/` directories removed (about 800 GB), 26 dead jj workspaces
forgotten, branch `S4.4` deleted. Still to delete once `ci-fixes`
lands: branches `S5`, `S6`, `S7`, `P6`, `merge-queue`, the `wb-merge`
worktree; `rescue/p4-invproj-20261026` holds nothing `main` lacks
(owner's call).

## 2026-10-28: the drift review and the merge plan

**Where things stand.** `main` is `44ee4083`. Since the hand-off of
2026-10-26, nine units landed in two days: U0c (`198aa2a1`), P7
(`3e65c83f`), P13 steps 1-3 (`579d0eed`), P4b S4.1 (`13282cf8`), S4.2
(`bf91c5a7`), U0a (`45396bdc`), U0b (`26d3a994`), P9a (`c8541ea9`) and
S4.3 (`d19fb464`); the owner made D-55 (the party contact block). Five
more units are built to their Done-when lists on branches the owner
paused on 2026-10-27 to free the machine, none merged: S4.4 (`002fb9f8`),
S5 (`ad649be3`), S6 (`6c4480cd`), S7 (`4b4fecb9`), and P6 checkpointed as
`ce366408` (feature complete, its gate interrupted, "not for merge" as
is). Not started: S4.5, P4, P5, P8, P10, P11a, P11b, P12, P13 steps 4-6.
The live books are unchanged since the P3 cutover and the
`document_standing` upgrade of 2026-10-24; `documents_v2` waits for Q-5.

**The 2026-10-28 review.** The planning session read the whole plan and
every design document against `main`, the five branches and dry-run
merges; the result is `plan/2026-10-28_DRIFT_REPORT.md`, and the plan
and docs were brought up to date in the same change (served.md section
0, "What changed on 2026-10-28"). The gate on `main` at `44ee4083`:
green (`exit 0`, 70 minutes on a loaded machine: fmt, clippy and
rustdoc for both workspaces, 167 test suites, 2,871 tests passed, 0
failed, 7 ignored, the server-only build with no window libraries).
The five branches' gates are being re-run in their worktrees, one at a
time, and each is recorded in 8.0.1 when it lands.

**What the review judged.** Every departure the workers recorded was
reasonable and is kept (the drift report lists them with the reason);
three are owed back (S4.3's missing journal transaction across a
multi-line command; S6's `confirmed`, deferred to P4's sweep; S6's
three bookspec examples); one needs a decision (P6 made `ledger-store`
depend on `ledger-import` so the writer can extract at attach time).
No golden moved, no kernel arithmetic left `kernel.rs`, no unit touched
the owner's books.

**The merge hazards the pause left** (dry-run merges, nothing touched):
every P4b branch created `crates/ledger-cli/tests/documents.rs` from
8.0.3's one allocated file (add/add on every merge after the first);
`types.rs` appends collide; P6 branched before S4.3 and took schema 16,
which S4.3 holds (S5 took 17), so P6 retakes 18; S5 and S6 both numbered
their law 34 in `docs/verification.md` (S1's documents law is now 34 on
`main`; S5 becomes 35, S6 36, S7's write-off 37). The order and the
resolutions are in served.md 8.0.2, "Resuming the paused wave".

**The destination (D-56).** The owner, 2026-10-28: the work is
"ultimately taking it to a single bundled server+GUI installation for
Mac/Windows/Linux and then beyond". The installed app (P11a, then P11b)
is the plan's destination and the hosted service (served.md section 11)
what follows. Today the executable is Linux-first: tokens live in the OS
keyring and only the Linux Secret Service store exists, so on macOS
`wanderbooks connect` and the agent bridge fail (`tokens.rs`,
`NOT_YET`), Windows is WSL2 only, and no macOS or Windows build runs in
CI. P11a gains step 0 for exactly that groundwork and starts now, beside
the P4b finish; the P10 chain (P4, P5, P8) keeps its order.

**Answered 2026-10-28 (the question tool):** Q-5, one format upgrade after
S7 and P4 (D-57); Q-8, D-47 to D-54 stand (D-58); the planning session
does the merges itself; the website is redeployed with the macOS wording.

**Next, in order:** (1) resume the five branches in the 8.0.2 order,
each rebased and `just check`ed again (the pause killed every gate),
P6 last with its gate run to the end and its dependency reviewed; (2)
ask the owner Q-5 and Q-8 when S4.4 lands; (3) S4.5; (4) P11a step 0
and the packaging trial, staffed now; (5) P4 (adding `confirmed` to the
settlement tools too), P5, P8, P10; P13 steps 4-6 beside; P11b after
P11a; P12 last.

**Housekeeping found:** about 1 TB of `target/` directories across the
six checkouts (1.7 TB free); 27 registered jj workspaces whose folders
are gone (`jj workspace forget`); branch `rescue/p4-invproj-20261026`
holds nothing `main` lacks (its bookspec example landed) and can go
after a last look.

## 2026-10-26 to 2026-10-27: the hand-off and its first wave

**2026-10-26: the plan was reviewed and rewritten for a team of engineers
working in parallel worktrees.** A nine-stage Elephant-Goldfish review
(`plan/review-2026-10-26/`, consolidated in
`consolidated_review_report.md`) found `plan/served.md` sound as a design
record and not executable as a hand-off: no remaining unit could be built
from the text alone, P7 contradicted D-28/D-29, P4 was never reconciled
with the P4b slices built after it, every code pointer was stale, and
nothing stated status, dependencies, file ownership or done criteria.
Section 8 of the plan is new: 8.0 (the status and dependency table, the
merge order, the shared files and allocated names, the eight-field unit
template, the recipes R-A to R-G, the journal-safety checklist, how to
work in a worktree), 8.1 (what landed, with As-built blocks for P4b
S1-S3), 8.2 (every remaining unit in merge order: U0a, U0b, U0c, P4b S4
to S7, P4, P5, P6, P7, P8, P9a, P10, P11a, P11b, P12, P13), 8.3 (the
corrected P4b reference design; the owner's answers numbered A-1 to
A-14), 8.4 (what is not scheduled, and why). Decisions D-47 to D-54 and
open questions Q-4 to Q-7 were added; superseded sentences in sections 1
to 7 and 9 are struck through with the decision that superseded them.
The hand-off prompt for the implementing team is `plan/HANDOFF.md`.
Nothing in the code changed. **Next: U0a and U0b (the pre-steps), then
P4b S4 with S5 and S7 beside it (8.0.2).** The first owner questions in
line are Q-5 (one upgrade after S7, or an earlier one after S4) and Q-4
(the release manifest, before P11a's signing step).

**2026-10-26: the team's first wave is open; U0c landed (`198aa2a1`).**
Eight units started in parallel worktrees: U0a (split the MCP server by
tool group), U0b (the attention engine over `PackageBooks`), U0c, P4b
S4.1 (terms, numbers, line accounts), P4b S4.2 (settings and party
fields), P7 (the TIN scanner's separators), P9a (the self-host recipe)
and P13 steps 1-3 (the bookspec checker). U0c is the first to land: the
three missing `docs/testing.md` sections (P4b S1 part B, S2, S3), each
with its For-P12 paragraph, and the A/R and A/P section of
`docs/mcp-interface.md` checked against the shipped tools. It changed
no code and no golden. It found a code gap, recorded in U0c's As-built
block and here: `books.documents`' `party` filter matches a customer's
or vendor's name, never the party id, while the parameter's schema
description offers an id; the doc now says name, and the schema or the
handler needs the fix in a unit that may change code (P4b S4.4 or P5's
surface pass). The baseline `just check` on `main` before this wave was
green.

**2026-10-26: P7 landed (`3e65c83f`).** The TIN scanner now reads the
commas, parentheses and any run of spaces a TIN's groups are written
with (`123,45,6789`, `(123) 45-6789`, `12    3456789`), while an
amount's comma-grouped thousands (`1,234,567.89`, 1-3-3-2) and the
company's nine-digit D-U-N-S number stay what they are. A space run
counts once however long; a comma or a parenthesis joins only a 3-2-4 or
2-7 group. The old tests that pinned the narrow separator set were
replaced, not deleted: `has_full_tin_never_forgets_a_tin` is a property
over the old scanner (any TIN it found, the new one finds), and the
truth table gained the comma, parenthesis, space-run, amount, D-U-N-S
and phone cases. `tin_shapes_with_any_separator_are_found` generates the
separators; the access log and the filing scan have their own tests, and
the TIN canaries in `spec_vault.rs` still pass. Replay is unchanged
(B-28), so old journals open as they did; vault v2 stays closed (D-48).
Under this machine's parallel load (load average ~55)
`closing_during_a_burst_of_posts_through_the_bridge_loses_nothing`
flaked twice ("nothing posted"); it passes alone and passed on `main`
quietly, and it is unrelated to the scanner. P12 or the test's own unit
should make that burst test load-tolerant rather than let the gate depend
on an idle machine.

**2026-10-26: P13 steps 1-3 landed (`579d0eed`); the unit is in
progress, not done.** bookspec now has its own Cargo workspace in the
repository with the checker (the example schema and parser, every
structure `deny_unknown_fields`, the cross-field checks, the rule
catalogue) and the runner, plus the `Engine` trait and the wanderbooks
adapter over `Store`. `crates/ledger-core/tests/bookspec.rs` is the
adapter and still checks all 56 examples; the runner reports per rule,
and a rule with no example is `NotClaimed`, never a silent pass.
`docs/spec/rules.md` carries a `**bookspec:**` id under each of its 38
stated rules; `bookspec/spec/rules-index.md` maps B-1 to B-45, with
B-39 to B-45 allocated and unstated. 15 rules are claimed today because
15 have examples; step 4 owes an example for every rule, the normative
text and sources, and the five laws as Lean theorems; steps 5-6 owe the
spec's own checker and the GnuCash adapter. `just check` now runs the
bookspec workspace's fmt, clippy, rustdoc and tests beside the main
workspace's, and was green at the merge.

Two departures, both reviewed. The trait gained a required
`Engine::change(&ChangeSpec)` (the plan's four methods have no path for
an example's `[[changes]]`, and skipping those examples would have
rejected part of the corpus). And the rule catalog's `DECISION` status
is now `CHOICE`, bookspec's word: `docs/spec/rules.md`, `AGENTS.md` and
`docs/spec/README.md` say so, and the rebased P7 sentence that still
said DECISION was fixed in the same landing. **Every later unit that
states a rule must use CHOICE where a human chose and must add its
`**bookspec:**` id line** (B-39 S4.5, B-40 S4.5, B-41 S7, B-43 P4,
B-44 P6, B-45 P8); `rules_md_carries_every_id` fails the tree otherwise.

**2026-10-27: P4b S4.1 landed (`13282cf8`); P4b S4 is in progress.** The
core can now carry a document's terms, its number, its line's own account,
quantity and unit price, the parties' snapshots and the bill's vendor
reference: `Terms`/`TermsKind`/`EarlyDiscount` with exact due dates
(Net n, Net n EOM, due on receipt, custom), `kernel::discount_max` and
`kernel::line_amount` (Kani, with bridge properties),
`PartySnapshot`, `Invoice`/`Bill`'s new optional fields,
`Ledger::next_invoice_number` and `Ledger::check_vendor_ref`. Ten tests in
`spec_documents.rs`; `just check` and `just prove` green (51 fast Kani
harnesses, Lean 90 headline theorems, standard axioms only); no golden
moved.

One guard, and the reason it exists: `Store::create_invoice` and
`create_bill` refuse a line carrying `account`, `quantity` or
`unit_price` (`StoreError::DocumentFieldsAwaitUpgrade`) because the
journal does not write those fields yet. A store caller could otherwise
post a document whose replay silently loses its line account, which I-3
forbids and no MCP path reaches today. **S4.3 removes the guard in the
same change that journals the three fields and adds its byte-for-byte
test**; `document_line_fields_wait_for_documents_v2` proves the refusal,
the unchanged journal and the unchanged ordinary posting, and the guard
sits before `rehearse` in both commands. Departures to carry forward: the
`Sent`/`SentVia`/`SendKind` types are defined here though S5 owns them
(**S5 adds `Ledger::mark_sent` and must not redefine them**);
`InvoiceFields`/`BillFields` and `create_invoice_with`/`create_bill_with`
are the carrier for the create inputs the plan names but does not place;
and `check_document_line_account`'s fallback to the store's name-based
cash-account rule is a second copy of that rule, to unify later. The
bookspec examples `payment-terms-due-date` and
`invoice-numbers-run-and-voids-keep-theirs` remain owed by a later S4
slice, since P13's checker now takes an unknown key as an error.

**2026-10-27: P4b S4.2 landed (`bf91c5a7`).** The invoicing, bills and
company-contact settings are journaled keys (eleven of them) with the new
`Kind::{Count, Terms, PaymentMethods}`, and a party can carry an email and
default terms as customer and as vendor. `documents_v2` exists early —
its constant, `KNOWN_FEATURES` entry and `documents_v2_is_its_own_feature`
landed here because the party and settings writes must call
`ensure_feature`; **S4.3 must not redeclare them**, and it owns the
document write call sites, schema 16 and the byte-for-byte test. The
branch's stub `Terms` was deleted at the rebase; `arap.rs` is S4.1's
alone, and `settings::Kind::Terms` canonicalizes through `Terms::parse` /
`describe`. `LedgerError::FirstNumberLocked` exists as the contract only:
**S4.3 enforces it at `set_setting`**. The 8.0.6 work is in the branch: a
hand-written new-shape line in `books_written_under_older_rules_still_open`,
the `p4_s42_settings_parties` old-books fixture written by `1bfce915`, the
`invoicing.default_terms` refusal in `books_p2_served_cut_over_whole`, and
`documents_v2_is_its_own_feature`; no figure or golden moved. One test
adaptation, not a weakening: `terms_keep_one_canonical_spelling` now
asserts that `Due 2026-1-5` is accepted and stored as `Due 2026-01-05`,
which is S4.1's `Terms::parse`.

**2026-10-27: U0a landed (`45396bdc`).** `server.rs` is the router and the
request machinery (1,909 lines, no `#[tool(..)]` handler); the 98
handlers it held live in fourteen `crates/ledger-mcp/src/tools/*.rs`
groups, each with its own `tool_router`, and the helpers several groups
share sit in `tools/helpers.rs`, re-exported by `server.rs`, so no group
file reaches into another's. The surface is identical: the tool table is
122 (the unit's 118 is stale), `annotations_complete` passes, and a
one-off dump of every tool's name, title, description, schemas and
annotations in `list_all()` order diffed empty against `main` before the
split. The whole-surface tests stay in `server_tests.rs` beside
`server.rs`; each group's own tests moved with it. `just check` was green
on the merge candidate, which included S4.2. **From here, every unit adds
MCP handlers only in its own group file** (`documents.rs` for S4.4/S6,
`drafts.rs` for P4, `evidence.rs` for P6, `audit.rs` for P8); `server.rs`
keeps the router and the shared helpers.

**2026-10-27: U0b landed (`26d3a994`).** Needs attention is one engine in
`ledger-package::attention`, read by the viewer and by `books.start`; the
viewer's module only maps a target to a route. The review found and the
unit fixed a parity gap: the server had passed `Integrity::default()`
("healthy") while the viewer computed its own, so a damaged book showed
the `database` or `stored_files` item to the human and not to the agent.
`books.start` now runs the real check, and
`the_viewer_and_books_start_see_the_same_attention` proves the two lists
equal item for item on a clean demo, a written book, that book with a
missing receipt blob, and that book with a tampered projection. Cost
measured on the demo books: 138 distinct blobs hashed, `books.start` cold
287 ms / warm 256 ms in a debug build on a loaded machine; no cache was
added, and the As-built records that a book with thousands of large scans
is where one would go. `Attention` also carries the prompt (the viewer's,
so there is one copy) beyond the plan's sketch, and `PackageBooks` gained
four read methods rather than a new trait. `just check` green.

**2026-10-27: P9a landed (`c8541ea9`).** The self-host recipe is in
`deploy/` (a system unit as the `wanderbooks` account, `ProtectSystem`,
`NoNewPrivileges`, loopback-only network, no seccomp/Landlock per D-32),
`books adopt` moves a book with its head check, `wanderbooks serve
--isolation enforced` makes `books.start` say `service_user (enforced)`,
and `--http` serves the same MCP door on loopback behind Caddy or a
tunnel, with the bearer still guarding every call. The door is
**loopback only**: a review found the first version bound any address,
which would have exposed a plaintext static-bearer door; it now refuses
anything but 127.0.0.0/8 or `::1` before a socket or a host exists, and
`the_tunnel_door_binds_loopback_only` proves it. `two_user.rs` has the
two-account tests; `agent_uid_cannot_read_tenant` skips unless
`WB_TWO_USER=1` and the CI job sets it. **Done-when (1) the CI job and
(2) the clean-VM walkthrough by a non-author are owner/EM to verify** —
they were not run here, and the As-built says so. `IPAddressDeny=any`
also blocks the server's own push to a remote backup repository; the
README documents the `IPAddressAllow` drop-in. One pre-existing race was
seen under load, not ours: `ledger-gui/tests/newer_journal.rs::a_live_viewer_says_update_when_the_books_move_ahead_of_it`
can read between the test's two appends; it passed alone and in the
final run, and belongs with the other load-sensitive test noted above.

**2026-10-27: P4b S4.3 landed (`d19fb464`); the document journal now
holds the new fields.** `invoice.created`/`bill.created` carry number,
terms, snapshots, references and each line's account, quantity and unit
price — only when set, so an old-shape document is byte-for-byte the same
(`no_new_fields_journals_byte_for_byte`). S4.1's interim guard is gone;
`documents_v2_waits_for_the_upgrade` proves the write is refused before
consent and accepted after; schema is 16; `FirstNumberLocked` is enforced
at `set_setting`; the issued PDF is filed in the same call as the entry,
`invoice.created`, `receipt.attached` and the link to the invoice
(`issued_pdf_is_filed_with_the_invoice_or_nothing_is`), with the SHA-256
the attachment's `blob_sha`; `preview_invoice`/`preview_bill` journal
nothing; and the old-books fixture `p4_s43_documents` opens to the same
figures. No figure or golden moved; `just check` green; no `just prove`
needed (no new `Ledger` command, only replay support).

Two departures recorded in the As-built. The unit's "a failure anywhere
leaves the journal unchanged" is true of **every refusal** (all are
prevalidated; the PDF's bytes are stored first), but the store has no
transaction over journal appends — `atomically` does not exist — so an
I/O failure between the invoice's lines and the attachment's is not
rolled back; that is a property of every multi-line command, not a new
one, and a journal transaction is a larger change. And the plan's schema
16 named `parties.email` and two terms columns, but the projection has no
parties table; replay and `differs_from` carry those facts, and inventing
a table with no reader was rejected. Books that have not declared
`documents_v2` still issue with the legacy `Invoice {id}.pdf` and no
number (8.3.14 item 5), so an issue works before the go-ahead. **S4.4
owes** the over-the-wire consent entry for a document carrying the new
fields and the tool docs; **S4.5 owes** B-39/B-40, parity, the S4
For-P12 paragraph, the handbook and the two bookspec examples.

**2026-10-27: the owner chose the full party contact block (D-55).** The
question was raised because S4.3's schema 16 skipped the plan's
`parties.email`/terms columns: the SQLite projection has no parties
table, so those facts were never in a table — they live in the journal
(`party.updated`) and are replayed and compared by `differs_from`, so
nothing was missing. The owner's ask is a real extension: a party now
carries `phone`, `contact` (a named person), `billing_address`,
`shipping_address` and `billing_email` besides S4.2's `email` and the two
term sets; `PartySnapshot` captures them at issue, the invoice prints
the billing ones and a Ship-to line when shipping is set, and
`books.update_party` / `books.create_invoice`'s `bill_to` take them. It
folds into `documents_v2` because the owner's upgrade has not happened,
so there is no extra go-ahead. **S4.4 builds core/store/journal/MCP; S4.5
the PDF.** Terms per side were already there and are unchanged.

**2026-10-27: the owner paused the team to free the machine.** All five
agents (S4.4, S5, S6, S7, P6) were told to stop, and each acknowledged and
ended its turn; their in-flight checks and detached `setsid` jobs were
terminated, so resuming means re-running `just check`, not waking a paused
process. Branches and worktrees are intact and clean: S4.4 (`002fb9f8`),
S5 (`ad649be3`), S6 (`6c4480cd`), S7 (`4b4fecb9`). P6 had no commits, so
its whole change set is checkpointed as `ce366408` (`wip: pause
checkpoint — P6 feature complete, gate interrupted (not for merge)`).
Nothing was merged, rebased or discarded; the last landed commit is
`ed637f0e`.


The 2026-10-16 audit of the first company's books (62 verified findings,
20 engine defects) and the design discussion that followed produced
`plan/served.md`: the books become a server that owns its data, agents
reach them only through the MCP from anywhere, settings and conventions
move into the journal, no scaffold. Phases P0-P13; the owner's decisions
of 2026-10-16 are at the top of the plan and bind every phase. P0 turned
up defects 21 (the CPA package's IIF ignores `as_of`) and 22
(depreciation's final month plugs mid-month); both are for P3.

The owner's bar (2026-10-17): this is bookkeeping software and it has to
be perfect. Full verification every phase (review rounds until clean,
`just check`, `just prove`, goldens); economize only on waste. Mutation
testing waits for P12 from P2 slice 3 on (D-33, owner 2026-10-20: focus
on building; P12 reruns the whole max-testing pass, on the owner's desktop
or CI).

**P0 (guard rails): done.** On `main` at `391634a`, pushed (origin +
GitHub): pins, transports doc, tools-only template; the store's format
gate (newer is not corrupt; the envelope and its hash frozen across
formats; no repair of newer books; command vs replay validators, B-28);
the short connect text and `books.handbook`; tool annotations; the import
roots' lockdown (Downloads only by default); fail closed; no file with a
full TIN filed into the books. Four review rounds, the last clean.
Cutover on the first company's books, 2026-10-17: `books.verify` ok
(366 events, head unchanged), and the CPA package as of 2026-10-11 and
2026-10-17 canonically byte-identical before and after (record in that
books dir, `goldens/p0-cutover-2026-10-17/`). Connect text with their
settings: 1,396 characters.

**P1 (journal-native settings and conventions): landed.** Settings,
conventions (posting templates, approval rules, policies, close steps,
calendar, mappings, notes), decisions, open items and approvals are
journal events (format 2); books that keep their settings never read
`wanderbooks.toml` or `config.json`, and adoption moves a file in
exactly once (`books.import_settings` did too, until D-44). Two review rounds (all findings fixed), `just
check`, `just prove` (Kani 20/20, Lean 31 theorems, standard axioms
only), and mutation on the whole diff (956 mutants; every survivor
killed by a test but two equivalent: `docs/testing.md`). **Live on the
first company's books 2026-10-17:** settings imported (20; the time zone
set after), its conventions, decisions 1-13 and open items 1-7 recorded,
and every company document filed (D-20). The cutover found two defects,
both fixed in `94338b7`: the K-1 worksheet printed a share read from the
journal at the scale it was written ("0.5", not "0.50"), and the file
guard refused documents printing the company's own EIN. On the rebuilt
server `books.verify` ok (434 events) and the CPA package as of
2026-10-11 and 2026-10-17 canonically byte-identical to before, and to
the P0 goldens (record in that books dir, `goldens/p1-cutover-2026-10-17/`).

**Before-snapshots for later phases:** the first company's reports as of
2026-10-11 and 2026-10-16 (journal head `d1ccd2fd…`, 355 events) are
saved in that books dir under `goldens/p0-2026-10-16/` (exported through
the MCP, no TINs), privately, by decision, never in this public repo
(plan 5.6). P1+ must reproduce them except where the P3 change log
adjudicates a difference (cash flow, defect 1).

**P2 (one executable, open while held, the cutover): built, slices 1-8.**
Slices 1-2 on `main` earlier. Slices 3-8 (2026-10-19 to 2026-10-20),
each reviewed until a round came back clean: 3, opening and closing the
books (the server on local sockets, the host and its leases, the agents'
bridge; no background service, D-21/D-23); 4, access (tokens in the
keyring, the door, scopes, the access log, the owner passphrase); 5, files
in and out through the server (uploads, exports); 6, the viewer on the data
root and one CPA-package builder (`ledger-package`); 7, backups and restore
(D-30, D-31: plain git to a remote and a folder, drills,
`wanderbooks books restore`); 8, the viewer's Books menu, `connect`,
`books back-up`, no confinement (D-32), the scaffold deleted. Then the
tests mutation had found before the pause (D-33), with three real bugs
fixed: the admin answer's four-MiB line, and the CPA package's file
naming (a twin's number in another folder; a name of dots climbing out of
its folder). `docs/testing.md` lists what P12 must cover.

**P2 live on the first company's books, 2026-10-20 (slice 9).** Asked one
at a time: the old copies move aside (Q-3), backups go to a new private
repository on the owner's git server and to his Syncthing folder (Q-1,
D-30). Adopted as "fernhill" at event 466, 66 stored files and the TIN
vault checked; `books.verify` ok on the new server with the same head; the
CPA package as of 2026-10-11, 2026-10-17 and 2026-10-20 canonically
byte-identical before and after (record in that books dir,
`goldens/p2-cutover-2026-10-20/`); both backups whole by a restore
preview. The cutover added `wanderbooks books put-aside` (three review
rounds of data safety), which moved the old copies whole into the private
folder's `legacy/`. Agents connect with `wanderbooks connect`, the viewer is
`wanderbooks gui fernhill`, and the books dir is a books home holding no
books data. The before-check found defect 21 again (the IIF ignores
`as_of`).

**P3 (engine correctness): live since 2026-10-22.** Every defect group of plan P3 is on `main` (`2c5b5a7`), each
built from its brief, reviewed until a round came back clean, and with
its regression tests written as bookspec examples (51 in
`bookspec/examples/`, every one checked by `cargo test`): the odd cent
(defect 17, D-10, D-38), the IIF in plain ASCII and the CPA package as of
one day (defects 7, 21, D-36), the cash flow statement by ASC 230's
direct method (defect 1, D-35, D-42), rename, one set of period defaults
and tax categories that warn or refuse (defects 15, 16, 8), party totals
by what moved the money (defects 14, 11, 24), dated 1099 decisions,
payment methods and the card exception (defects 23, 12, 18, D-40),
depreciation straight-line by days through the final month and daily
schedules (defects 22, 9, D-37), the format upgrade only with the owner's
go-ahead (D-39), and assets' posted and scheduled depreciation, their
bases and the K-1 on the tax basis (defect 13, D-41); then three review
commits: the seams between the pieces (one fictional year checked
across engine, viewer, tools and package; three defects found and
fixed), asset basis (schema 14, shipped migrations pinned by hash; the
tax K-1's net income always open), and data safety (three new journal
features, replay with P2's TIN detector, the P2 served-books fixture). Mutation waits for
P12 (D-33): each piece's targets are listed in `docs/testing.md`.

**The P3 surface review (2026-10-21)** checked the agent's surface, the
docs and the whole proof run on `main` with all of P3: `just check`
clean; the tools' docs (`docs/mcp-interface.md`) corrected tool by tool
against the code (about 60 fixes: wrong parameter names, missing outputs
and refusals, the idempotent list, the handbook's section list,
`books.start`'s next steps, the CPA package's files) and a new test that
every tool is documented and every documented tool exists
(`every_tool_is_documented_and_every_documented_tool_exists`; 115 tools,
`books.access_log` was undocumented); B-rule cross-references fixed in
code and tests (eight wrong); bookspec's statuses made consistent (an
example whose figures follow an owner decision is CHOICE and cites it:
thirteen statuses changed, five more examples given the decision they
rest on), its README now saying exactly what the loader takes,
and the loader refusing a closed-list value spelled any other way. **`just
prove` could not finish in its 30-minute cap at four jobs** (a CI
runner's cores) on a heavily loaded laptop: 52 of 53 harnesses in, the
reversal mirror at three lines still running. It is split, nothing
dropped: that harness is proved in two halves (`kernel::entry_flows` is
now `entry_movements` then `flows_of`) plus end to end at two lines, and
`just prove` runs 47 harnesses and Lean (308 s at four jobs on a quiet
machine; 83 headline theorems, standard axioms only) while `just
prove-slow` runs the eight slowest, each under its own cap (467 s), and
CI runs both (`docs/verification.md`, "Running it"). The laws are numbered
1 to 31 with no gaps, every harness listed with its runtime. The viewer's
P3 pages were rendered on the demo books and looked at
(`~/.cache/wanderbooks/shots-p3-surface/`): the figures tie across
pages (the register, the asset, the balance sheet, the tax summary, the
1099 worksheet and the party agree to the cent); two faults fixed: a
report's notes were cut to one line (the fixed asset register's lost its
explanation of the tax columns; they wrap now), and the Accounts page
listed accounts in the order they were added, not by code.

**The P3 cutover (2026-10-22).** The first company's books were reinstalled
from `main` (`3eda95e`: P3, its four reviews, Projects with its snapshot,
and the keyring fix). Before the owner's go-ahead, every report was compared
with the P2 goldens at four dates: the statements, ledger, journal,
members' equity, K-1 worksheet and tax summary identical; the cash flow,
asset register, IIF, 1099 worksheet, README and the new tax-basis K-1 file
different, each difference adjudicated twice in the private change log
with every changed figure recomputed, none unexplained. The owner ran
`wanderbooks books upgrade` (a verified backup first); after it the
packages were byte-identical to before it, `books.verify` ok, both backups
whole by a restore preview. Then the P3 facts were recorded: the devices'
values and §723 bases with the traditional method (D-41), every vendor's
1099 decision (D-40), the payment methods with every member- and
card-paid entry tagged, the creditors of 2050 and 2100, and
`policy.require_tax_category = refuse`.

**Projects (2026-10-22, live with P3).** A project is a class with facts
(B-37), four tools, and a project page in the viewer opening on a
snapshot: revenue, direct costs, contribution before overhead and its
margin, in the black or red, the on-spec cost unrecovered, by month with
the net to date, and costs by category. The sidebar measures its footer.

**The keyring (2026-10-22).** One keyring item per token, read back when
made; the bridge says a locked keyring instead of "make a token"; `token
check` tells a kept token live, revoked, missing or duplicated.

**Invoices and bills take a project (2026-10-22).** Each line of
`books.create_invoice` and `books.create_bill` takes an optional `class`
(a project, by id or name): the entry's revenue or expense side is one
line per project, the untagged lines' as one, the receivable or payable
line and the payments never tagged (rules.md B-37 item 8), so a client
project's invoiced revenue and a contractor's billed cost count in its
figures, the P&L by class, and its snapshot; the P&L, the 1099 worksheet
and party totals don't move. Journaled as the line's `class_id` behind a
new feature, `document_line_projects` (not `projects`, which the live
books already declared); a document naming none is journaled byte for
byte as before. Late-tag notes as `books.add_line`'s; `books.get_entry`
shows a line's project; the viewer's invoice and bill pages list the
document's lines, each project a link; the demo's "Client work" carries
its invoices. The handbook's caveat is gone for books with the feature.

**One way to serve books (2026-10-23, D-44).** The owner: "We should
probably just delete the stdio path altogether. THere's no point in that.
And it sitting there unused will just lead to bitrot. Nuke it?" Gone:
`wanderbooks dev-stdio`, the `ledger-mcp` binary and `ledger_mcp::run`
(stdio and `--http` over a bare data dir), with what only it had: files
the books make coming back inline (every handed file is a download now),
books with no access log or backups, the settings file read and reloaded
(`legacy_files`), the vault key named by a file or `--vault-key-file`.
`BooksServer` is built one way, from a book as the books server serves it
(`Served`); an unknown tool is a protocol error. Every suite that drove
the stdio server moved to `crates/ledger-cli/tests/` and drives a real
`wanderbooks server` on a data root of its own (`tests/common`), files by
their download. Inline file input was unchanged then (D-46 removed it,
below). The owner then decided
(2026-10-23, the question tool): `books.import_settings` goes (118
tools; old books come in whole by `books adopt`; `books.chart_template`
gives its settings as `books.set_setting` changes); the viewer's
loose-books-folder mode goes (`--data-dir`, `--books-toml`,
`--vault-key-file`, the settings file followed): it opens only books of
the data root; and `wanderbooks dev-stdio` goes with no stub, an unknown
command like any other.

**Files only by handle (2026-10-23, D-46).** The owner, via the question
tool: "Uploads only"; and before it: "What we would DEFINITELY want to
avoid is passing a bunch of BLOBs through AI context windows. That's just
a pure waste." No file passes through an agent's context, either way. In:
`content_base64` and its `sha256` (`books.attach_receipt`,
`books.file_document`), `w9_content_base64` and `w9_sha256`
(`books.set_w9`) and `csv_text` (the two CSV imports) are gone with the
256 KiB inline limit; files come only by `books.begin_upload` and
`wanderbooks upload`; those tools take no other argument, and an inline
argument to any tool is refused before it runs as unknown, pointing to
`books.begin_upload`. Out: `books.export` (every format),
`books.asset_labels` and `books.get_receipt` view `bytes` hand the file
over as a download (`HandedFile`), as the CPA package and the W-9 did; a
token keeps up to 64 handed files (was 16). The tests file by upload
(`tests/common`: `Wire::upload`, `upload_on`) and read every file by its
download (`docs/testing.md`, "D-46").

**P4b slice S1, receivables and payables read their figures from the
ledger (2026-10-24, D-45).** Part A (ledger-core, store, viewer, Lean
`control_agrees`, Kani bounds): an invoice's or bill's paid, outstanding
and status are derived from its tagged lines that still stand, so a
reversed payment no longer counts and a void is exact; accounts in the
receivables or payables role (or holding document lines) refuse hand
lines; format feature `document_standing` (the owner upgraded
2026-10-24). Part B (MCP): `books.void_document`, `books.control_check`,
`instrument` and `reference` on `books.receive_payment` and
`books.pay_bill` (status and remaining from the derived figures), the
tie-out in `books.verify` and `control_problems` in `books.digest`; the
over-the-wire test `documents_void_and_tie_out_over_the_wire`. Still for
later slices: the processor fee on a payment (S6, with multi-document
payments and discounts), Needs attention in the viewer for the control
check, and S2 to S7. S2 (2026-10-24): `books.documents`,
`books.document`, and the viewer's Invoices & Bills (Money in, Money out,
a document's page, overview tiles). S3 (2026-10-24): the invoice PDF
(`ledger-import` `invoice_pdf`, `templates/invoice.typ`; `ledger-package`
`invoice_document`): re-created from the books, each payment on its own
line with its date, method and reference, stamped PAID with the day,
PARTIALLY PAID, PAST DUE or VOID (DRAFT, gray, for S4's preview); by
download from `books.document pdf`, and Save invoice PDF in the viewer.
The copy issued is kept from S4 on and never changes.
**Still open after P3 (each for the phase named, or the lead):**
- Invoices and bills take a project now (above), behind the new journal
  feature `document_line_projects`: the first company's books declared
  `projects` but not it, so until the owner's next format upgrade
  (`wanderbooks books upgrade`, D-39) their invoices and bills still take
  none (the handbook says so).
- Raised by the cutover review (private change log, "Open points"):
  its three points are done. The third (2026-10-22, B-36 with D-38): the
  tax-basis K-1's odd cents of property basis no longer all go to the
  first member; they run cumulatively, D-38's largest remainder, one
  count per set of members with a part in an entry, in date order
  whatever the span, a reversal taking its original back out
  (`tax_capital::RunningSplit`, `entry_parts`; tests
  `tied_odd_cents_of_property_basis_run_cumulatively`,
  `a_member_outside_an_entry_never_takes_its_cents`,
  `an_entrys_odd_cent_is_the_same_in_every_span`,
  `the_split_runs_in_date_order`, the law
  `a_running_split_never_drifts_a_cent`; bookspec
  `partnership-k1-tax-basis-odd-cents`). Left as it was, and said in
  B-36: the book property each member contributed in an entry is still
  split entry by entry, ties to the first member (each member's property
  must stay within their own credit in the entry); its odd cents arise
  only in an entry part cash and part property whose credits don't
  divide the property evenly. Whether the first company's three-cent
  gap between its members came wholly from the basis split is to be
  checked on its books when the K-1 is next run, at the year-end close.
  Its second point is done (2026-10-22, B-14 rule 5): a payment to a
  government (ASC 230-10-45-17(c), "Cash payments to governments for
  taxes, duties, fines, and other fees or penalties") is other operating
  cash payments, (g), never cash paid to suppliers, decided by its payee
  (a party with the government or tax agency role) on any account, as
  well as by a `taxes_licenses` account as before; a payment shared with
  a supplier is split by the parties' shares (45-22); payroll
  remittances from the payroll payables stay (d) as before; a government
  selling goods or services is a supplier (add it as a vendor). The
  first company's state filing fee, booked to its organization-costs
  account, moves from "Cash paid to suppliers and employees" to "Other
  operating cash payments" at every date (no total changes), provided
  (1) its entry names the state's agency with the `government_agency`
  role (check with `books.party`, give the role with
  `books.set_party_roles` if not), and (2) the fee's own entry is the
  one whose cash the statement lists: paid from the bank; charged to a
  card that is a lender's credit (the `financing_accounts` setting, D-42,
  borrowed and spent); or paid by a member personally through the
  members' account (Due to Shareholders) while that account is in
  `financing_accounts` (B-14 rule 11: borrowed from the member and spent
  on the fee's date, the operating part classed (g) with its
  `on_behalf`, and split the same way when shared:
  `a_shared_government_payment_a_member_paid_splits_both_ways`). The
  first company's fee was paid the last way (the books home's goldens
  change log, entry #1), so check its members' account is a financing
  account. Charged to a card, or paid through a members' account, that
  is an operating payable, its cash leaves in the later payment (to the
  bank, or the member's reimbursement), whose payee is not the state,
  and it stays with suppliers: check how it was paid (`books.get_entry`).
  Expected golden change: once this ships, the cash-flow snapshots in the
  books home's `goldens/` (each cutover's) differ from a fresh
  run on two operating lines, "Cash paid to suppliers and employees"
  smaller and "Other operating cash payments" larger by the fee, every
  section total, net change and cash balance the same. That is this
  rule, not a regression: diff the class lines against it at the next
  cutover and refresh the snapshot then.
  Its first point is done (2026-10-22, B-34 rule 8): the IIF writes each
  line's project in CLASS, the projects declared first in a `!CLASS`
  list, their names held to QuickBooks' 31 characters, no two alike, and
  listed in the README when written differently (tests
  `iif_writes_each_lines_class_and_declares_the_classes_first`,
  `iif_class_names_fit_quickbooks_and_every_change_is_listed`,
  `iif_class_names_written_as_the_books_have_them_are_kept`,
  `iif_refuses_an_undeclared_or_unfit_class`, the property
  `iif_class_names_never_clash`, `the_package_iif_carries_each_lines_project`;
  bookspec `iif-quickbooks-journal-file`). The first company's
  package changes at its next build: its project-tagged lines carry
  the class, and the class list is new.
- The viewer recomputes a long-life asset's whole schedule every frame
  (`ledger-gui` `pages/assets.rs` `schedule`, called from the asset
  page's draw; defect 22's note: slow, never wrong).
- A schedule month a run leaves for a human (dated in a closed period, or
  reversed after it ended) has nothing to mark it booked by hand, so its
  schedule never shows complete; a schedule or an asset registered by
  mistake has no way to stop. Both want a design.
- A reversal dated before what it reverses is refused for depreciation
  runs and schedule postings only (B-9); every other entry still takes
  one: a general rule to settle (P4's reversal as a draft is the natural
  place).
- Sources not yet read or vendored (`docs/spec/README.md`, "Open"): ASC
  606-10-25-23 (DART only), 25-31, 25-32, 55-20 (B-16); 230-10-45-15(f)
  (B-14 rule 11); the ASC 250 paragraph B-15 rule 6 means; 606-10-55-39
  (B-17); 105-10-05-1 to 05-3's current wording. Public domain but not
  vendored: IRC §701 and §6225 (B-14 rule 5), Treas. Reg.
  §1.704-1(b)(2)(iv)(d)(1) and (g)(3) (B-25, cited only). Several cited
  pages in `sources/intuit-iif.md` carry no SHA-256, and
  `irs-entity-classification.md` and `file-names.md` no Source, Retrieved
  or SHA-256 lines; Unicode UAX #14 (B-34) has no source file; Rev. Proc.
  98-25's excerpt (`irs-records.md`) carries no "§5.01" label.
- Eight tool descriptions are over 700 characters (listed in
  `tool_descriptions_fit_700_characters`): shortened in P5.
- The every-order asset search runs three commands deep in `just check`
  (four and five clean by hand, `WB_ORDERS_DEPTH`); six is P12's. The
  `ledger-store` test
  `ops::tests::the_server_makes_the_folder_and_the_viewer_only_reads_it`
  failed once in a whole-workspace run (likely a forked git child holding
  the viewer lock's descriptor); it passed in this review's runs.

**Next:** the P3 cutover, then P4-P13 as planned. P11: one app for
Windows, Mac and Linux (D-24). P12: max testing again (D-33). P13:
bookspec (D-34), a formal, sourced accounting specification first, then
its own formal validation and a conformance harness for any engine; P3's
regression tests are its first examples.

---

# Earlier status — 2026-10-11 (all 10 natural next steps done; repo now public)

## Where we are

All ten natural next steps are done, tested, and pushed. The repo is now
public. Parity is 12 of 12 against the spreadsheet workflow.

**Done:**
- Repo `swaits/wanderbooks` on git.swaits.com (public as of 2026-10-11).
- `ledger-core` 0.1.0: `Money` (exact Decimal + ISO currency), five account
  types with normal sides, chart of accounts (codes, hierarchy,
  placeholders), journal entries with full validation, `Ledger` command
  surface (create/add-line/post/reverse), append-only posting log.
- Reports: trial balance, P&L, balance sheet (accounting equation holds;
  current earnings folded into equity pre-close), canonical JSON `snapshot()`
  for oracle diffing.
- 14 tests green: validation rejections, posted-entry immutability,
  reversal semantics, the accounting laws on a fixture company, a 300-entry
  randomized invariant test, and the first golden file.
- `ledger-store` 0.1.0: the source of truth is an append-only, hash-chained
  JSONL journal (fsync per append; tamper-evident); content-addressed
  SHA-256 blob store (immutable, deduplicated, hash-verified on read);
  SQLite as a *disposable* projection (TEXT amounts, WAL, `synchronous=FULL`,
  `integrity_check` on open). Single write path: validate → journal →
  mutate → apply. `verify` replays the journal and diffs reports three ways;
  `rebuild` drops SQLite and replays; a missing database heals on open.
  7 tests green: round-trip, tamper detection, torn-tail truncation,
  rebuild, blob dedup/corruption detection, unbalanced-post rejection.
- `ledger-mcp` 0.1.0: stdio server on official rmcp 3.5 (23 narrow typed
  tools, amounts as decimal strings, `initialize.instructions` carrying the
  agent operating contract). Draft → add lines → preview → confirmed post;
  server-side receipt policy (strictly greater than $25, per-account
  overrides, typed `receipt_required` rejection); digest; search; CSV/JSON
  export; period P&L and as-of balance sheet; fixed-asset tools (register,
  schedule, list, run-depreciation, dispose with preview). End-to-end stdio
  protocol tests green (initialize → tools → draft → preview →
  receipt_required → attach → post → reports → digest → reverse; and a full
  asset flow: register → schedule → idempotent runs → disposal preview →
  receipt_required → bill-of-sale → confirmed disposal → gain posted).
- **Fixed assets** (`ledger-core` + `ledger-store` + `ledger-mcp`):
  fixed-asset register (description, purchase date, cost, salvage, useful
  life, linked cost/accumulated-depreciation/expense accounts), straight-line
  book depreciation with day-prorated partial months and a final-month
  true-up so the lifetime total is exactly (cost − salvage), idempotent
  monthly runs, disposal with auto-posted final depreciation and gain/loss.
  Tax depreciation stays with the CPA. Tests: 10 asset unit tests
  (ledger-core), 6 store tests, 7 property tests, MCP e2e.
- **Testing initiative** (in progress): `proptest` property suite on
  `ledger-core` (trial balance always balances, A = L + E, reversals restore
  zero, posted-entry immutability, JSON round-trip, depreciation exactness —
  the last caught a real month-length-drift bug, fixed with the final-month
  true-up); `cargo-mutants` run on `ledger-core` (228 mutants); `cargo-fuzz`
  targets (journal-line parsing, entry validation, op sequences) scaffolded.
- 120+ tests green workspace-wide; `cargo fmt` clean; `cargo clippy
  --workspace --all-targets` clean, zero warnings (a hard rule: the owner,
  2026-10-11).
- Docs: AGENTS.md (operating manual), requirements, architecture, oracle
  design, parity catalog (floor: 10 done), glossary, mcp-interface (incl. the
  5 asset tools), receipt-store (journal design), capability-parity
  (10 of 12 rows covered). Plan: this file + roadmap.
- **Formal verification** (2026-10-13, `docs/verification.md`): the
  money arithmetic moved into one generic kernel (`kernel.rs`) that the
  engine calls. 18 Kani harnesses prove it exhaustively within bounds,
  with covers against vacuity and a canary for a Kani soundness bug. 24
  Lean 4 theorems prove the accounting laws for books of any size, after
  any sequence of commands. A property test bridges `i64` and `Decimal`.
  `just prove`. Found and fixed: negative line amounts accepted by the
  engine, an overflow crash on huge amounts, and an overflow crash in FX
  conversion.

**In progress:** nothing — the next arcs are queued below, unstarted.

## Next arcs (in order)

The project went AI-first on 2026-10-10 (the owner's call): the primary
interface is MCP, not a cloned GUI. The first storage + MCP slice is done
(arcs 1–3 below). What remains:

1. ~~**`ledger-store`**~~ done — journal + blob store + SQLite projection.
2. ~~**Blob store**~~ done — folded into `ledger-store`.
3. ~~**`ledger-mcp`**~~ done — 18 tools, receipt policy, digest, export.
   Remaining from the spec: MCP *resources* (rmcp 3.x has no resource
   macros; hand-roll `list_resources`/`read_resource` next), Streamable HTTP
   transport, XLSX/IIF export formats.
4. **Capability parity** (`docs/capability-parity.md`): 11 of 12 rows
   covered; full CPA export (xlsx/iif) remains. Then the month-long parallel
   run against the GSheet; cutover per the cutover rule.
5. Then: oracle harness (cross-check diffs, GnuCash first), read-only GUI on the event log,
   workflow parity.

## Decisions log

- 2026-10-23: D-46, files only by handle: no inline file bytes in or out;
  uploads in, downloads out (plan/served.md D-46).
- 2026-10-23: D-44, one way to serve books: the stdio server, its inline
  file outputs and its settings-file mode are gone; tests drive a real
  books server (plan/served.md D-44).
- 2026-10-13: Formal methods are part of the bar (the owner: "This shit is
  fucking GOLD"). Money arithmetic lives in `kernel.rs` with Kani
  harnesses; accounting laws get Lean theorems; `just prove` stays green.
  The engine now rejects negative and out-of-range line amounts (max one
  quadrillion, ten decimal places). Every dev tool is pinned in
  `mise.toml`. See AGENTS.md "Formal methods".

- 2026-10-10: Single currency per ledger for day one (R-5). Multi-currency
  is designed-for, not built.
- 2026-10-10: `Money` arithmetic is checked and returns `Result`; no
  panicking ops, no float paths anywhere.
- 2026-10-10: Negative zero is normalized away at `account_balance` — money
  never prints "-0". (Caught by the golden test on day one; the oracle works.)
- 2026-10-10: License MIT OR Apache-2.0, matching the storytold projects.
  GnuCash clean-room boundary documented in `docs/oracle.md`.
- 2026-10-10: AI-first pivot (the owner). Primary interface is MCP
  (`docs/mcp-interface.md`), not a cloned GUI. Receipts/documents become a
  content-addressed blob store (`docs/receipt-store.md`). Parity target for
  the phase is the GSheet+GDrive system (`docs/capability-parity.md`), with
  a parallel-run cutover rule. GUI drops to read-only, event-log-driven,
  phase 4.
- 2026-10-10: Import/export targets QuickBooks formats (CSV, IIF, XLSX) —
  "could not give a single shit about GnuCash XML." GnuCash stays a
  *behavioral* cross-check only. (2026-10-13, the owner: it is "an" oracle,
  not "the" oracle; never trust it to be correct.) Receipt enforcement is server-side policy in the
  MCP layer (threshold + per-account overrides, typed `receipt_required`
  error, checklist in every preview); the engine stays policy-free.
- 2026-10-10: Storage durability (the owner). SQLite is the query engine, not
  the source of truth: the source of truth is an append-only, hash-chained
  JSONL journal (fsync per append; tamper-evident; git-backup-friendly),
  SQLite is a disposable projection rebuilt from it, blobs stay
  content-addressed. Single write path: journal → fsync → apply to SQLite.
- 2026-10-10: Draft durability (house decision). Drafts are **durable**, not
  ephemeral: `entry.created` + `entry.line_added` journal lines record the
  draft as the agent builds it, so drafts and their attached receipts survive
  restarts. Drafts never touch the SQLite projection until posted. Rationale:
  the journal is the source of truth (phantom `entry.created` lines without
  draft lines were the worst of both worlds), and the draft → attach receipts
  → post workflow spans multiple MCP calls.
- 2026-10-10: Corrupt SQLite heals itself. If the projection fails
  `integrity_check` (or isn't a database at all), `Store::open` quarantines
  it to `wanderbooks.corrupt.<timestamp>` and rebuilds from the journal,
  instead of failing open. WAL/SHM are dropped as derived state.
- 2026-10-10: Cash flow statement (R-FUTURE-3) done. Engine
  `reports::cash_flow` (indirect method) + `books.cash_flow` MCP tool (24
  tools total). Disposals identified structurally; gains/losses move from
  operating to investing; the three sections always reconcile to the net
  change in the designated cash accounts. Two invariant tests, including a
  full purchase → depreciate → dispose cycle.
- 2026-10-10: MCP resources done (hand-rolled `list_resources` /
  `read_resource`: chart, trial-balance, balance-sheet, profit-and-loss,
  digest, receipts). `#[tool_handler]` must sit on the `impl ServerHandler`
  block, not the inherent impl — misplacing it silently drops all tools.
- 2026-10-10: QuickBooks interop (R-FUTURE-4) done. New `ledger-import`
  crate: QBO bank CSV parse (3-col and 4-col layouts), chart CSV parse,
  IIF journal generation, XLSX report generation (rust_xlsxwriter, MIT).
  MCP: `books.import_bank_csv` (drafts + categorization rules),
  `books.import_chart_csv` (all-or-nothing), `books.export` gains `xlsx`
  (base64) and `iif` formats plus `cash_flow` report. 26 tools total.
  Capability parity 12 of 12 rows covered.
- 2026-10-10: Streamable HTTP transport done. `ledger-mcp --http ADDR`
  serves MCP at `/mcp` via rmcp's tower StreamableHttpService (axum).
  27 tools total. Verified with curl: initialize returns SSE stream.
- 2026-10-10: Drive backfill path done. `books.import_local_file` reads a
  file from the server's filesystem into the blob store (receipt/document).
  The agent downloads from Drive via the CLI, then calls this — Drive
  credentials never enter the server.
- 2026-10-11: `wanderbooks init` done. New `ledger-cli` crate (binary
  `wanderbooks`, std-only): `init [DIR]` scaffolds a books dir — `mise.toml`
  (pinned Rust toolchain; tasks: build, mcp-add, backup), `AGENT.md` (agent
  operating contract: bootstrap, MCP contract, first-run onboarding incl.
  migration via conversation, ongoing rules), `wanderbooks.toml` (company
  profile, filled during onboarding), `books-data/` + `.gitignore` (SQLite
  projection ignored; journal is the record). Fresh dirs get `git init` and
  a scaffold commit so the journal archive starts on day one. One generic
  init by design — no per-situation flags; the agent is the import layer
  (QBO/CSV/email/sheets, conversational). 5 integration tests green; clippy
  clean. Bootstrap: `mise trust && mise install`, `mise run build`,
  `mise run mcp-add`, then `claude` in the dir.
- 2026-10-11: Receipts are mutable through events, never edits (the owner:
  "the journal is just a history of every single thing that happens").
  `books.unlink_receipt` and `books.withdraw_attachment` (both require a
  reason) journal `receipt.unlinked` / `attachment.withdrawn`; nothing is
  erased from the journal or blob store. Unlinking can't strand an entry the
  receipt policy requires evidence for (attach the replacement first).
  `link_receipt` now refuses an already-linked or withdrawn attachment, which
  closes a silent path for moving a receipt off a posted entry; disposals
  check their receipt ids before posting. Schema v5. First real use: swapping
  the migrated the first company `.eml` receipts for their PDF renders.
- 2026-10-11: The cash flow statement is a first-class output everywhere the
  other two statements are: `cash_flow_pdf` in ledger-import, a `pdf`
  format on `books.export` (P&L, balance sheet, cash flow), and
  `cash-flow.pdf` in the CPA package, which now requires `cash_accounts`
  (the statement reconciles to them). Known gaps, next: the P&L and balance
  sheet PDF/XLSX ignore start/end/as_of (all-time balances under the
  requested label; core needs period-scoped statements); P&L and balance
  sheet XLSX are totals-only; PDF amounts don't align (padding in a
  proportional font), no company name, negatives as "-x".
- 2026-10-11: Every statement export (csv, json, xlsx, pdf, CPA package)
  now renders core's period statements: the date range and as-of date are
  real, with a line per account. One layout (`ledger-import::report`:
  sections, lines, subtotals, totals; exact "(1,234.56)" formatting) feeds
  every renderer. The server reads `wanderbooks.toml`: company name for
  report headers, `fiscal_year_start` for fiscal YTD (end derived), and
  `receipt_threshold` (it was silently ignored before). Reports carry
  "Prepared from journal through seq N".
- 2026-10-11: PDFs are typeset with Typst (typst 0.15 via typst-as-lib,
  fonts bundled by typst-assets; printpdf removed). One template,
  `ledger-import/templates/statement.typ`, takes the laid-out statement as
  JSON data (labels are never markup). US accounting presentation in PDF
  and XLSX alike: tabular figures, "$" on each section's first amount and
  on totals, negatives in parentheses with the closing paren hanging so
  decimal points align, zero as a dash, single rule over sums, double rule
  under totals, amounts never bold (bold figures are wider). Pages flow
  with a repeated header and "Page n of m". Core's cash flow now orders
  lines by account code (it followed HashMap order, so line order could
  change between runs).
- 2026-10-11: The canonical report set. Core gained the general ledger by
  account (running balances; QuickBooks' "split" = the opposite side of the
  entry, "-Split-" when several accounts), the trial balance as of a date,
  and the statement of changes in members' equity (capital roll-forward per
  member, allocation by share to the cent, ties to balance-sheet equity).
  The entry-ordered listing is now correctly called the general journal.
  Reports render QuickBooks-style (the owner's reference): Inter (bundled,
  OFL) instead of the typst-assets serif fonts, centered title block, a
  column-header row, multi-column statements (trial balance Debit/Credit,
  one column per member), "$" on sums, rules above sums, a double rule
  under totals, zero as 0.00, entity terms (Members' equity, Net income
  (loss), standard cash-flow wording) and an "Accrual basis · Prepared …"
  footer. "Today" is local time, not UTC (evening exports were dated
  tomorrow).
- 2026-10-11: Period close fixed and made partnership-aware. The close swept
  every revenue/expense posting ever made, so a period closed after later
  entries existed swallowed them (closing January in March closed February
  and March too); it now zeroes balances as of the period end, refuses to
  close out of order, and its preview is the real close run on a copy of
  the ledger. A multi-member LLC closes into each member's capital account
  by share (CloseTo::Members), the partnership convention, so capital
  accounts carry allocated earnings; the members'-equity statement treats
  those closes as income, not contributions, and the balance sheet shows
  only unallocated earnings. Replay now checks closing lines, not just ids.
- 2026-10-11: Tax categories (the owner: wanderbooks is a general MCP-first
  accounting application, so features ship for every small business, not
  when the first company needs them). Accounts map to stable categories rather than
  form line numbers; each category knows its returns and default
  deductible percent; the tax summary shows book vs. tax by category and
  every uncategorized account. tax_form comes from config or the tax
  classification. Schema v6.
- 2026-10-11: Owner equity for every entity type. Chart templates for sole
  proprietorships, partnerships/multi-member LLCs, S corps, and C corps
  (equity and owner-pay accounts, tax categories, matching config). Closes
  sweep draws, distributions, and dividends into their target; partners'
  distribution accounts count as capital on the members' equity statement
  and close into it. CloseTo became RetainedEarnings { sweep } / Account
  { into, sweep } / Members([MemberClose]); journals without a target, or
  with the old plain "retained_earnings", still replay.
- 2026-10-11: Schedules. Prepaid expenses and deferred revenue amortize
  monthly on month ends, to the cent; runs are previewed, idempotent, and
  exempt from the receipt policy (the schedule is the evidence). Recurring
  entries generate drafts, never posts. Journal events schedule.registered,
  schedule.posted (with the full entry, checked on replay),
  recurring.registered, recurring.generated. No schema change. 59 tools.
- 2026-10-11: Payouts. books.record_payout grosses a platform deposit up
  into revenue (by class), fees, and adjustments, refusing unless it
  reconciles to the cent; it creates a draft so the payout statement is
  attached as the receipt. Pure core logic (payouts.rs), no new journal
  events (it journals as a draft and lines). 60 tools. Statement parsers
  (Apple, Google, Stripe) wait for sample files.
- 2026-10-11: 1099 tracking. Vendors get form_1099 (NEC/MISC) and
  tin_on_file (never the TIN); entries get an optional vendor tag (bills,
  bill payments, and reversals inherit it). books.vendor_1099 counts cash
  paid per vendor per calendar year against $600 (≤2025) / $2,000 (2026+,
  OBBBA) or an override, and lists reportable vendors missing a W-9.
  Journal events vendor.1099_set and entry.vendor_set. Schema v7. 64 tools.
  Also: requirements and parity no longer list finished R-FUTURE work as
  future (only budgets remain, declined).
- 2026-10-12: ledger-gui, the read-only desktop view (egui 0.36, wgpu).
  Overview (the answer in a sentence, KPI tiles with honest comparisons,
  cash chart, needs-attention with copyable agent prompts, recent
  activity), Cash (accounts, in/out, burn and runway, coming up), Profit
  (by month with margin, spending vs. its recent average, by customer and
  class), Transactions (search, filters, cash direction, detail with
  lines, the rendered receipt via hayro, and journal history), Reports
  (P&L, balance sheet, cash flow, owners' equity, trial balance, GL, tax
  summary, 1099, agings; date presets, month/quarter/year columns, prior
  period/year comparison, % of income, drill-down to the account register
  for exactly those dates, copy as TSV), Accounts and registers,
  Documents, Company, Journal (every event, hash, payload), Ctrl+K
  palette, back/forward, light/dark. Live: notify-driven, ~30 ms from
  write to screen. Engine additions: ledger-store Snapshot (read-only,
  incremental, torn-tail tolerant), ledger-core analysis (intervals,
  balance series and history, P&L by interval, flows), ledger-config
  (extracted from ledger-mcp; full profile, cash_accounts). Fonts: Inter
  with tnum frozen in and private-use glyphs removed (Phosphor icons).
- 2026-10-12: One-command onboarding. `wanderbooks new DIR` installs the
  server and GUI if missing, writes AGENTS.md (+ CLAUDE.md importing it),
  wanderbooks.toml, a justfile (agent, gui, backup, update), .gitignore,
  .mcp.json (the server registered for that dir only) and
  .claude/settings.json (approved, no prompt), commits, and starts claude
  with the onboarding prompt. `install.sh` does Rust + cargo install +
  `wanderbooks new` in one line. mise is no longer part of the scaffold.
- 2026-10-12: Integrity, end to end. Store::verify now also compares the
  whole SQLite projection (every table, row for row) with a fresh
  projection of the journal; verify_and_heal rebuilds a drifted database
  (keeping the old file) and runs at every server start; books.verify
  (65 tools) adds a re-hash of every blob. The GUI compares the server's
  database (read-only) with its own projection and re-hashes each file
  once, showing "Database matches journal · N files verified" and raising
  problems in Needs attention. Also: Windows-safe blob writes (no
  directory fsync off Unix), .exe-aware binary lookup, installer OS and
  C-compiler checks with WANDERBOOKS_REPO, the website (site/, `just
  site`, `just publish`), and a repo justfile (check, build, install, demo).
- 2026-10-12: The CPA story. The CPA package now also carries every
  receipt behind the period's entries (receipts/, with index.csv tying each
  to its entry, date, amount, and SHA-256), the company's documents, the
  journal as IIF (QuickBooks Desktop), the 1099 worksheet, and a README
  with the verified journal size; include_receipts=false for a light one.
  The scaffolded AGENTS.md teaches the bookkeeper to find receipts in
  email, migrate from QuickBooks/Xero/spreadsheets (match every statement
  to the cent; keep the old system read-only for a month), and work with
  the CPA (package, read-only clone + GUI). The site leads with owning
  your books and an AI bookkeeper: example prompts, a QuickBooks
  comparison, an honest "what it doesn't do yet", the CPA section, FAQ.
- 2026-10-12: The agent can drive the viewer. books.show (66 tools) asks
  the viewer to open an entry, account register, report (with dates),
  document, search, or page. The request lives outside the books (a JSON
  file in the runtime dir keyed by the data dir, written atomically); the
  viewer watches it beside the journal and applies books before
  navigation. ~30 ms end to end.
