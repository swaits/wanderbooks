# Drift report, 2026-10-28: the plan, the code, and the documents

Written by the planning session on 2026-10-28, after the implementing
team (an engineering-manager agent with worker agents in git worktrees,
2026-10-26 to 2026-10-27) was paused. Method: every plan and design
document in `plan/` and `docs/` was read whole and compared with `main`
(`44ee4083`), the five paused branches (`S4.4`, `S5`, `S6`, `S7`, `P6`),
their worktrees, dry-run merges (`git merge-tree`, touching no branch),
and the code. Only divergences are listed; what matches is not.

## Where we are, in one paragraph

Of the plan's remaining units, nine landed on `main` in two days (U0a,
U0b, U0c, P7, P9a, P13 steps 1-3, P4b S4.1, S4.2, S4.3) and five more are
built to their "Done when" lists on unmerged branches (S4.4, S5, S6, S7;
P6 is feature-complete with its gate interrupted). Not started: S4.5, P4,
P5, P8, P10, P11a, P11b, P12, P13 steps 4-6. The owner's destination,
stated 2026-10-28, is one installed app for Mac, Windows and Linux (P11a,
then P11b), then the hosted service of section 11; today the executable
is Linux-first (the keyring store exists for Linux only, so on macOS an
agent cannot connect), nothing of P11a exists, and no macOS or Windows
build runs in CI.

## plan/served.md

- **8.0.1 and the unit Status lines are stale.** The S4 row says "S4.4-S4.5 open"; S4.4 is built on branch `S4.4` (`002fb9f8`, 2 commits, 29 files) with its As-built block written. S5 (`ad649be3`, 10 commits, 44 files), S6 (`6c4480cd`, 4 commits, 45 files) and S7 (`4b4fecb9`, 12 commits, 49 files) say "not started" and are built, each with its As-built block, `just check` reported green by its worker on the branch rebased on `ed637f0e`. P6 says "not started" and is built to every step on branch `P6`, checkpointed as `ce366408` ("wip: pause checkpoint, P6 feature complete, gate interrupted (not for merge)"): its `just check` never finished.
- **The merge hazards of the pause are not recorded anywhere.** Dry-run merges onto `main` in plan order: S4.4 clean; then S5 conflicts in `crates/ledger-cli/tests/documents.rs` (add/add) and `crates/ledger-mcp/src/types.rs`; S6 in `documents.rs` (add/add) and `docs/mcp-interface.md`; S7 in `documents.rs` (add/add), `types.rs`, `crates/ledger-store/src/lib.rs`; P6 in `types.rs`, `store/src/lib.rs`, `store/src/schema.rs`, `store/tests/store.rs`, `docs/parity.md`, `docs/receipt-store.md`, `docs/spec/rules.md`, `docs/testing.md`. Each branch alone merges onto `main` cleanly except P6. Causes: 8.0.3 allocated one over-the-wire test file (`crates/ledger-cli/tests/documents.rs`) to four units, and each created it; the "append at the end" rule for `types.rs` collides whenever two units append; P6 branched before S4.3 and took schema 16, which S4.3 holds on `main` (S5 took 17), so P6 must retake 18 at its rebase. S5 and S6 both numbered their new law 34 in `docs/verification.md` (S5 `Step.markSent`, S6 `settle_is_exact`); S1's documents law, which was never written up, is 34 on `main` as of this report, so S5 becomes 35, S6 36 and S7's write-off 37 at merge.
- **8.0.3's allocated names drifted, with the EM's approval, and the table was not updated:** two new MCP groups, `tools/sent.rs` (S5) and `tools/settlements.rs` (S6, which also moved `books.receive_payment`, `books.pay_bill`, `books.ar_aging`, `books.ap_aging` out of `documents.rs`); `books.mark_sent`'s recipients parameter is `recipients`, not `to` (the `no_tool_takes_a_path` lint refuses `to`); P6 built `books.set_entry_settles`, named in its Build step but absent from the new-tools row (P6 flagged it).
- **S6 departed from its Build in ways the owner should see:** `confirmed` (preview then confirm) was not added to `books.receive_payment` and `books.pay_bill` (deferred to P4's one sweep); the processor's vendor on a fee is named in the entry's description, not tagged (tags are entry-wide); no schema change (18 not taken); the three bookspec examples were not written (two wait for journaled terms); the over-the-wire discount tests wait for S4.3's journaled terms, which have since landed.
- **P6 departed in a way that changes the crate graph:** `ledger-store` now depends on `ledger-import` (Typst, the fonts, hayro enter the store's dependency closure) so the writer can extract at attach time. `docs/architecture.md`'s dependency rules neither allow nor forbid it; it needs a decision at P6's review (the alternative is extraction in the server before the store call, with the backfill command doing the same).
- **S4.4 changes the live contract A-1 planned:** existing callers of `books.create_invoice` and `books.create_bill` now need `confirmed: true`; the connect text and handbook on `main` still say "Invoice, bill, payment and payroll tools post at once, unchecked", so that sentence is stale the moment S4.4 merges (S4.5 owns the handbook; the connect text is `handbook.rs`).
- **Q-5 is due and unasked.** Its trigger is "when S4 lands"; S4.1-S4.3 are on `main` and S4.4 is built. A-14 is still reserved for the answer.
- **D-47 to D-54 are the review's recommendations, never explicitly confirmed by the owner** ("the owner may revisit"); S5, S6, S7 and P6 were built on them (D-47, D-49, D-52, D-53). Confirming them is now a one-line question (Q-8 below).
- **P11a's Build omits the groundwork the design assigns to P11 elsewhere:** the macOS and Windows keyring stores (3.3 as built; D-19; `crates/ledger-host/src/tokens.rs` returns "kept in the keyring on Linux only, until the macOS and Windows apps (P11)" on every other OS, so `wanderbooks connect` and the bridge fail there), the Windows data root under `%LOCALAPPDATA%` with an owner-only DACL (3.4), named pipes with a SID check (3.9), and a CI build on macOS and Windows runners (none exists: all three workflows run `ubuntu-24.04`). None is a step in 8.2 P11a.
- **8.0.2's priority statement is stale against the owner's destination.** It names P10 as "the owner's goal" and leaves P11a as "any time"; the owner's statement of 2026-10-28 makes the installed app the destination, with P11a to start now beside the P4b finish (recorded as D-56).
- **Section 2 carries superseded sentences without the strike-through rule of section 0:** I-1 "Landlock and seccomp confine the server itself where available", I-2 "Landlock harness `no_tool_touches_outside_tenant`", I-8 "`gui_works_under_read_only_landlock`" (all → D-32).
- **`server.toml` (3.2, 3.4) was never built**; the deployment flags are `wanderbooks serve --http` and `--isolation` (P9a). The references read as if it exists.
- **4.9's "The tool count is 118 (2026-10-26)"** is 122 on `main` (U0a's As-built says so) and becomes 130 once the five branches merge (S5 +1, S6 +1, S7 +1, P6 +4, S4.4 +0).
- **8.0.6 item 2 fixtures**: S4.3 wrote `p4_s43_documents`, S5 `p4_s5_before`, S7 `p4_s7_before`, P6 `p6_evidence`; S4.4 and S6 wrote none of their own (S6 reuses `p4-arap-before`, which predates S1). Acceptable under the item's intent (no figure moves), recorded here so the EM does not look for them.

## plan/STATUS.md

- The title and the "Now" section are dated 2026-10-26 and say "P4b S4 next"; since then U0a, U0b, P9a, S4.1-S4.3 landed and S4.4, S5, S6, S7, P6 were built on branches. The pause paragraph (2026-10-27) is the last entry but the head of the file was not revised, so a reader is told the wrong next step.

## plan/HANDOFF.md

- "What is true today (2026-10-26)" says nothing after S3 is built. Nine units landed and five are on paused branches.
- No procedure for resuming paused worktrees: the merge order of the five branches, the known conflicts (above), the schema renumbering P6 needs, and that every branch's `just check` must be re-run after the rebase (the pause killed the gates).
- It does not say that P11a is to be staffed now (D-56).

## plan/roadmap.md

- Phase 2 says "27 tools" (122). Phase 3 (oracle harness) reads as not started; `just oracle` runs 200 random books through GnuCash 5.10 with 15,800 exact comparisons (docs/oracle.md Layer 5); the machine-readable parity catalog and CI floor are the part still open.
- Phase 5 lists A/R and A/P, bank reconciliation and period close as future; all three exist (reconciliation and close since 2026-10-11, A/R and A/P first-class since P4b S1). Multi-currency is partial (foreign lines; no translation), as it says.
- "Explicitly not on the roadmap: payroll" while `ledger-core::payroll` and the payroll tools exist (accounting only, no tax tables). The sentence means tax tables and filing; it does not say so.
- Nothing names the destination: the installed app (P11a, P11b) and the hosted service (served.md section 11).

## plan/gui.md

- "Status (2026-10-12)" predates: the Books menu (back up, restore, new books, connect, close; P2 slice 8), 80 color schemes (D-19's family menu), Invoices & Bills pages and document pages with timelines (P4b S2, S3), Projects pages, old-books compatibility tests, Needs attention computed by `ledger-package` and only routed by the viewer (U0b), the data root as the only source (`wanderbooks gui <book>`; `--data-dir` and `--books-toml` removed, D-44), the W-9 and TIN clipboard copy (D-29), the format-upgrade dialog (D-39).
- Principle 4 says the GUI "builds its own in-memory view" and offers prompts for the agent; the prompts now come from the shared attention engine.
- The "Next" list (cash forecast, reconciliation view, receipt zoom, keyboard navigation, virtualized lists, web build) was never revisited; the reconciliation page exists; the web build is the hosted service's viewer (section 11).

## README.md

- "118 MCP tools" twice (122 on `main`); "190+ workspace tests" (about 2,700 test functions across the two workspaces).
- The quickstart and the installer claim Linux and macOS. On macOS the executable builds and the viewer runs, but `wanderbooks connect` and the agent bridge fail: tokens live in the OS keyring and only the Linux Secret Service store is built (`tokens.rs`, `NOT_YET`). Windows is WSL2 only, as it says.
- "Status" says the Google Sheet stays the system of record until a clean month and cutover approval; the cutover happened 2026-10-20 (P2 slice 9) and the Sheet is frozen as of 2026-10-11.
- "Mutation testing via cargo-mutants" is paused for development until P12 (D-33; CI still runs its weekly shards).
- The layout omits `ledger-vault`, `bookspec/`, `verify/`, `deploy/`, `fuzz/`, `oracle/`, `site/`.

## AGENTS.md

- "`ledger-core` depends only on `rust_decimal`, `thiserror`, `chrono`, `serde`, `serde_json`": it also depends on `jiff` (the time-zone database for `company.timezone`'s "today", P1). The rule needs the sixth name or a justification recorded.
- The docs index omits `docs/testing.md`, `docs/parties-and-attachments.md`, `plan/served.md` (the plan every remaining unit is built from), `plan/HANDOFF.md`, `bookspec/`, `deploy/`.

## docs/architecture.md

- "Current crate layout" shows `ledger-core` alone with six files; the workspace has ten crates (plus the bookspec workspace of two), and `ledger-core` has about 25 modules. "Future crates (do not create until the roadmap says so)" lists `ledger-store`, `ledger-import`, `ledger-export`, `ledger-oracle` as future: the first two exist, `ledger-export` was folded into `ledger-import`, and the GnuCash cross-check is `tests/oracle_gnucash.rs` with `oracle/gnucash/`, not a crate. `ledger-config`, `ledger-mcp`, `ledger-cli`, `ledger-host`, `ledger-vault`, `ledger-gui`, `ledger-package` are described in later sections or not at all; there is no dependency graph.
- "the projection has a table for each (schema v12)": 16 (17 on branch S5).
- The `ledger-mcp` groups list will need `sent` and `settlements` when S5 and S6 merge (their branches add them).
- The P6 branch's `ledger-store` → `ledger-import` dependency is not covered by the dependency rules (see served.md above).

## docs/parity.md

- "**Floor: 10 done** (2026-10-10)" while the table holds 21 `done` rows.
- No P4b entry at all. S1 (documents read their figures from standing lines, the control guard, `books.void_document`, `books.control_check`), S2 (`books.documents`, `books.document`, the viewer pages), S3 (the invoice PDF) and S4.1-S4.3 (terms, numbers, line accounts, the issued PDF filed) landed without touching the catalog, against AGENTS.md's "every feature change updates `docs/parity.md` in the same commit". The A/R and A/P rows' notes still read "R-FUTURE-5".
- "Automated cross-implementation diff harness (GnuCash first) | todo" while `just oracle` runs it weekly (docs/oracle.md Layer 5); what remains is the fixture-book snapshot diff and the machine-readable catalog.
- "Workflow/UI parity ... gets its own catalog when the egui phase starts": the viewer exists; no such catalog was started (not a defect: the owner made MCP the primary interface, 2026-10-10).

## docs/capability-parity.md

- The cutover rule reads as pending. The cutover happened 2026-10-20 (P2 slice 9); the Sheet is frozen as of 2026-10-11 and the final `verify-sheet` run is a filed workpaper (served.md 8.4). The document should say so and keep the rule for the record.

## docs/parallel-run.md

- Entirely pre-cutover ("70 workspace tests"; "delete the wanderbooks data dir" as rollback). Historical since 2026-10-20; it carries no banner saying so.

## docs/requirements.md

- R-13 names `require_receipts_above` with a default of `"25.00"` enforced by the MCP layer; the policy is the journaled setting `policy.evidence.*` (P1, B-29), evaluated by `ledger-config`'s `ReceiptPolicy` today and moving into `ledger_core::evidence` in P4 (I-5 as amended by D-47).
- "CPA handoff package: ... P&L and balance-sheet PDFs (printpdf, MIT)": Typst since 2026-10-11, and the package holds the cash flow, GL, journal IIF, receipts, documents, the 1099 worksheet and the K-1 worksheets.
- "A/R and A/P workflows ... MCP tools for the full workflow" says nothing of D-45 (figures derived from tagged standing lines; hand lines on the control accounts refused) or of what `documents_v2` adds behind the owner's upgrade.

## docs/oracle.md

- Layer 3 (mutation testing) is "live now"; development mutation runs are paused until P12 (D-33), CI's weekly shards aside.
- Layer 5 both says the GnuCash differential is live and that "a `ledger-oracle` crate will run fixture books ... `cargo xtask oracle`"; there is no crate and none is planned (the harness is `tests/oracle_gnucash.rs`).
- "What the oracle does NOT cover: workflows ... a UI concern for the egui phase, where storytold's screenshot-in-the-loop comes back": the viewer exists with screenshot tests (`crates/ledger-gui/tests/*.rs`, `shot(..)`).

## docs/receipt-store.md

- "Migration from Google Drive" is written as future work; the company records were filed at the P1 cutover (D-20) and the Drive manifest and comparison tools were struck (served.md 8.4). The schema notes (v16, a file per schema) are current.

## docs/parties-and-attachments.md

- "Status: design; built after asset tax basis": built (parties, roles, the TIN vault, attachments to any subject, 2026-10-15 to 2026-10-17); D-55's contact block is built on branch S4.4.

## docs/mcp-interface.md

- The per-tool sections are held to the code by `every_tool_is_documented_and_every_documented_tool_exists`; no drift there. The framing is stale: "Crates in this phase" lists two crates and calls `ledger-store` "SQLite persistence" (the journal is the source of truth); "Muse" among the clients; "Out of scope for this phase". P5 owns the overview rewrite (its step 6).

## docs/testing.md

- The Layers table says `just prove` runs 48 harnesses and `prove-slow` 8; the code has 60 (`#[kani::proof]`), 51 in `prove` and 9 in the justfile's `kani_slow`.
- The mutation rows say "weekly" with no mention of D-33 (development runs paused until P12; CI's `scheduled.yml` still runs the shards).
- Units U0a and S4.1-S4.3 have no section; U0a added no tests (none owed) and S4's For-P12 paragraph is S4.5's by the plan, so this is deferred, not missing.

## docs/verification.md

- The harness table lacks P4b S1's two Kani harnesses, `bounds_check_matches_definition` and `bounds_kept_is_the_definition_on_books_in_bounds` (`kernel::running_bounds`, `bounds_kept`), and there is no section for S1's law (Lean `control_agrees`, `control_agrees_through`, `Reachable.control_is_documents` in `Documents.lean`), against AGENTS.md's "every harness and theorem listed". S7's branch amends the "out of scope" list; S5's branch adds rule 34 (`Step.markSent`).
- "Running it": 48 and 8 harnesses (51 and 9).

## docs/spec/README.md, docs/spec/rules.md

- No drift detected beyond the open sources already listed in 8.4 (B-38 to B-45 stay CONVENTION until vendored). The branches add B-41 (S7), B-42 (S6), B-44 (P6) and their bookspec ids.

## site/index.html, install.sh

- Both claim Linux and macOS. On macOS the install succeeds and the viewer runs, but `wanderbooks connect claude` (the installer's last step) fails on the keyring (see README above), so the one-line promise does not hold there until P11a's keyring stores. "Native Windows is on the way" is right. The site is deployed by hand (`just deploy`); the built copy under `site/dist` lags any source edit until then.

## deploy/README.md, deploy/wanderbooks-server.service

- No drift detected against P9a as built. Done-when 1 (the GitHub Actions job's first run) and 2 (the recipe walked on a clean VM by a non-author in under 30 minutes) are still the owner's to verify.

## bookspec/README.md

- No drift detected. Branch S7 extends the example schema (`write_off`, a refused `lines` entry); the README on that branch says so.

## plan/review-2026-10-26/

- Historical. No drift detected.

## Housekeeping found on the way (not plan drift)

- Build caches: about 1 TB under the six `target/` directories (`wanderbooks` 196 GB; `wb-S6` 220 GB, `wb-S5` 194 GB, `wb-P6` 167 GB, `wb-S4.4` 117 GB, `wb-S7` 109 GB) with 1.7 TB free. Once the branches merge, `cargo clean` in each worktree and `git worktree remove` recover it.
- 27 jj workspaces are registered (`p3-*`, `p4-*`) whose directories under `~/.cache/wanderbooks/` no longer exist: `jj workspace forget` each.
- Branch `rescue/p4-invproj-20261026` ("wip: rescue uncommitted p4-invproj work before cache cleanup, never merged", 51 files) sits beside `main`, which holds that work's landed form (`528c0706`, including its bookspec example `projects-invoice-and-bill-lines-count-by-project`); nothing on it is missing from `main`. Delete it after a last look.

## The deviations, judged

Every departure recorded in an As-built block was read against the unit's text and AGENTS.md:

- **Reasonable, keep:** S4.1's interim guard (removed by S4.3 as promised); S4.2 declaring `documents_v2` early (the settings writes needed the gate); S4.3 skipping the parties columns (no parties table exists; replay and `differs_from` carry the facts) and the `_with` carriers; S4.4 requiring the new bill fields only under `documents_v2` (8.0.4's "everything else works at once"); S5's Lean step for `mark_sent` (I-13 wins over the unit's "no Lean change") and the `recipients` rename (the lint is older than the unit); S6's `tools/settlements.rs` (the EM approved it to keep `documents.rs` single-owner) and the fee's processor in the description; S7's `least_owed_from` bound (B-38 invariant 2 at every later day, stricter than the unit) and the one-receipt-once rule; U0a's 122 (the unit's 118 was stale); U0b's extra `PackageBooks` reads and the integrity fix (a real parity gap found); P9a's loopback-only door (the review caught a plaintext static-bearer door bound to every address); P7 keeping the filing scan narrow (B-26's design); P13's `Engine::change` and the CHOICE vocabulary.
- **Reasonable but owed back:** S4.3's missing journal transaction (an I/O failure between the invoice's lines and its attachment's is not rolled back: a property of every multi-line command, to be fixed once for all in P12 or a small unit of its own); S6 deferring `confirmed` to P4 (right, since P4's sweep touches every confirm tool, but P4 must now add it rather than only `preview`); S6's unwritten bookspec examples (owed to P13 step 4 or S6's follow-up now that terms are journaled); the two S1 harnesses missing from `docs/verification.md`.
- **Needs a decision:** P6's `ledger-store` → `ledger-import` dependency (architecture), and P6's whole branch is a checkpoint whose gate never ran: it is not yet a reviewed unit.
- **Not reasonable, fix:** none found. No golden moved on `main`; no kernel arithmetic outside `kernel.rs` was introduced by the landed units; no unit touched the owner's books.

## Where to go next (the order)

1. **Resume and land the paused branches**, in this order, each rebased on `main` and `just check` (and `just prove` where it says) re-run after the rebase: S4.4 → S5 (resolve `documents.rs` by concatenation, `types.rs` by keeping both appends; schema 17 stands) → S6 (`documents.rs`, `docs/mcp-interface.md`) → S7 (`documents.rs`, `types.rs`, `store/lib.rs`; `just prove` for `writeOff_within_bounds`) → P6 (eight files; retake schema 18; run its gate to the end; review the store → import dependency). Then S4.5, the only unstarted P4b slice.
2. **Ask the owner Q-5 and Q-8** (served.md 10.3) when S4.4 lands; the `documents_v2` upgrade of the live books follows the answer (after S7 and P4 under D-52).
3. **Start P11a now** (D-56), beginning with its new step 0 (the macOS and Windows keyring stores, the Windows data root and pipes, a CI matrix on macOS and Windows runners), then the cargo-packager trial. P11a is independent of the P4b and P4 chain and is the owner's destination.
4. **Then the chain as planned:** P4 (now also adding `confirmed` to the settlement tools) → P5 → P8 → P10; P13 steps 4-6 beside; P11b after P11a; P12 last.
5. **Update the documents in this report's list** (done in the same change as this report where the fix is a fact, not a decision).
