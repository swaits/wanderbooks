# Stage 7: Critic review of `plan/served.md` (design, structural, style)

Reviewers: `egm-design` (feature-spec standard; scope P4b.3-13, P4, P5, P6, P8 against I-1..I-13 and D-28..D-46), `egm-structural` (whole document, emphasis §8-10), `egm-style` (§8 P4-P13, §9). 2026-10-26. Prefixes: DC = design, ST = structural, SY = style. Elephant verification notes in [brackets].

## Blockers

### DC-A1. `outstanding(d, D)` is defined over "entries that stand on D"; that double-counts re-reversals and contradicts the shipped code
P4b.3 1106-1113; P4b.13 INVARIANTS 1-3; Lean `control_agrees`. Origin +100, reversal −100, re-reversal +100: summing standing entries gives +200, the GL gives +100. [Verified: `Ledger::doc_state` (ledger.rs:5595) sums every tagged line dated ≤ as_of regardless of standing; `stands_on` is used only for the Void flag. The code is right; the plan text is wrong.] Fix: `outstanding(d, D) = Σ amount over posted lines tagged d with entry.date ≤ D`; `stands_on` only for Void; state the Lean theorem over the date-sum.

### DC-A2. Bounds and reversal checks run at creation; under P4 the reversal posts later, so a void can post under a payment
P4b.3 1130-1135; P4 974. `check_item_bounds` and `check_document_reversal` run in `check_reversal` when the reversal is created. If P4 makes a reversal a draft, a payment recorded between draft and post leaves outstanding −100. Same hole for schedule/depreciation checks in `Store::check_reverse_entry` and the un-posting side effects in `reverse_entry_steps`. Fix: posting an entry with `reversal_of` re-runs the whole reversal check at post time; a reversal draft is immutable (`add_line`, `set_line_class`, `set_entry_party` refused); Lean `Step.postReversal` separate from `Step.post`.

### DC-A3. The built immediate void/bounce is not reconciled with "corrections are reversal drafts that need evidence"
I-4, I-5, P4 971-979 vs P4b.6 1246-1247, P4b.7 1275, P4b.8 1316, owner answer 1. Either `void_document` bypasses the gate (contradicts I-5) or voids become two-step (contradicts answer 1). Fix: decide in writing: `void_document {confirmed}` creates and posts the reversal in one store call when the evidence requirement is satisfiable in that call (`link_original`: the issued PDF is the original's evidence; a bounce needs the bank's returned-item notice by `upload_id`); otherwise it returns a draft awaiting evidence. The draft step copies `DocTag`s and sets `reversal_of` before any line exists.

### ST-F1. No consistent per-unit shape; status, dependencies, ownership, done criteria absent for every remaining unit
§8 766-1889. Three shapes coexist. Only one "Status" line exists (997) and it is stale. Fix: the template below for P4, P4b S4-S7, P5-P13; a status/dependency table at the head of §8.

### ST-F2 / SY (also Z-2). P7 contradicts D-28 and D-29 and carries no marker
P7 1697-1722; also 2.1 row 84, I-7 (67), §6 line 671, top decision 5 bullet (19), D-16. Fix: rewrite P7 under the template; the `/h/` TIN pages move to P9 (remote only); strike "Removed from the MCP"; mark I-7, 2.1, 671, D-16 "changed by D-28/D-29".

### ST-F3. No file-ownership map or merge order for files every unit edits
`scopes.rs`, `server.rs`/`tools/*.rs`, `format.rs` KNOWN_FEATURES, `schema.rs`, `tests/format.rs`, `Ledger.lean` `Step`, `proofs.rs`, `settings.rs`, `handbook/*.md`, the six `docs/*.md`. Schema numbers cannot be allocated independently. Fix: "8.0.3 Shared files and merge order": owner, append-only reserved blocks per unit, feature names and schema numbers allocated up front.

### ST-F4. P4 was written before P4b and never revised against S1-S3
P4 971-993 vs P4b.3 1104-1109, P4b.8 1285-1307. P4 names no feature flag for four new event types; says nothing about tag copying on reversal drafts, draft-time vs post-time bounds, `stands_on` vs drafts, `void_document` on a draft. Fix: rewrite P4 with "Depends on P4b S1-S3 (landed)" and those integration points; name its feature.

### ST-F5. P4b status and slice list stale and self-contradictory on upgrades
997; 1004 "two upgrades"; 1265-1272 six features; 1281; 1629 "six go-aheads"; 1477. If upgrades are batched after S1 and S7, S4-S6 writes refuse until S7 lands; never said. Fix: per-slice Status; "Declared on the owner's books" column in P4b.7; state the batching rule and its consequence once; each slice its own H4.

### SY-B1 / ST-F13. "decision N" (N > 5) references resolve to nothing
1236, 1281, 1297, 1301, 1493, 1629. Answer 5 is an unnumbered run-on of nine items; any enumeration a reader reconstructs is off by one against the references. Fix: number the owner's answers A-1..A-14 (one per line) and rewrite each reference; reserve "decision" for D-n.

### SY-B2. The override sentence (997) tells readers to resolve contradictions themselves
Known conflicts it covers: `payment_methods` list vs `payment_instructions` string (1002 vs 1215); plural exempt keys vs "that key only" (1002 vs 1215/1631); `Sent` without sender/reference vs answer 4 (1003 vs 1185, 1325); fee vs no fee (1003 vs 1300/1304). Fix: delete the sentence; edit the later text to match the answers.

### SY-B3. `Snapshot` names two Rust types
1165 (party snapshot) vs the books `Snapshot` (1128, 1418, 1549, 1762). Fix: `PartySnapshot`.

### SY-B4. Broken find-and-replace: "the the first company", "`~/Documents/the first company-Books`" (1819, load-bearing in P11), 1778, 1646, 1657, 1694, 1721 (and 85, 405, 805, 841, 969). Fix: one placeholder, `<Company>` in paths, "the owner's company" in prose.

### SY-B5 / SY-B6. P9 1759 and P11 1801 each carry six or more instructions and an exception in one breath. Fix: tables (unit | IPAddressDeny | IPAddressAllow | …; format | built by | installs as | updated by | signed by) and one instruction per bullet.

## Major

### Design (DC)
- **A4.** The ordinal rule for multi-document payments breaks once discounts add a second control line per document; rules.md B-38 item 1 states a different rule ("first untagged control line for the amount"); "needs no feature" rests on an older binary's behaviour. Fix: under `settlements`, one event per entry with explicit line references `settlement.posted {entry_id, applications: [{line, doc, movement, amount}]}`; positional inference only for legacy single-document events; multi-document payments behind the same feature.
- **A5.** The processor fee (answer 4) has no posting row, argument, feature, role, kernel function or party rule; replay's line-by-line check would fail to open books with a fee line unless behind a feature; the entry-level customer tag misattributes the fee; `payouts.rs:178` already computes `gross − fees` outside the kernel. Fix: in `settlements`: `receive_payment {…, fee?: {amount, account?, vendor?}}`; posting row; `kernel::settle(applications, discounts, fee)` with Kani (and `payout_plan` calls it); role `payment_processing_fees`; fee line carries the processor's vendor tag or is a second entry in the same store call.
- **A6.** "0 ≤ outstanding at every date" refuses a real temporary overpayment (replacement payment received before a bounce is known); the only escape is mis-dating. Fix: `receive_payment` takes `excess_to: <liability holding no role>` (2300) in S6, or the plan says what the agent does.
- **A7.** A purchase discount on a bill with an asset line goes to expense, overstating the asset's cost and tax basis. Fix: apportion the discount by `kernel::apportion` across line accounts (asset lines reduce cost), `purchase_discounts` for the expense portion only; or refuse discounts on bills with asset/prepaid lines in S6 and name the remedy.
- **A8.** `adopt_control_line` can only create origins, so a legacy hand payment line becomes a phantom credit-balance document; an adopted document's `total` is undefined (`doc_header` uses `lines.total()`). Fix: `as: invoice | bill | payment_of: <doc> | discount_of | write_off_of`; an adopted origin gets one synthesized line (`account: None`).
- **A9.** The identity is per account but `outstanding` is per document; after `move_balance` a document's lines straddle two accounts and the Lean theorem is ill-defined. Fix: `owed_on(A, d, D)` per account; `GL(A, D) = Σ_d owed_on(A, d, D) + unattached(A, D)`; `outstanding(d, D) = Σ_A owed_on`.
- **A10 / B2.** The "single evidence gate in `Store::post_entry` (every path)" covers only drafts: documents, payments, payroll, payouts, depreciation, closes post inside `Ledger` commands; the GUI string-matches the origin label (`ledger-gui/src/books.rs:708`). Fix: `ledger_core::evidence::requirement(&Ledger, &JournalEntry, origin) -> Required | Exempt{source}` with `EntryOrigin` derived at replay from the owning event; one private `Store::evidence_gate` called from every Store command that posts; at-once tools take `upload_id|attachment_id`; move `ReceiptPolicy` into `ledger_core::settings`; GUI calls the same function over the Snapshot; delete the origin-string heuristic.
- **A11.** Recovery as "reverse the write-off, dated now" misstates Form 1065 lines for a prior-year write-off (§111: gross income, not a negative deduction). Fix: `Movement::Recovery`: Dr 1200 [d, Recovery] / Cr a `bad_debt_recoveries` revenue role; `written_off` nets Recovery.
- **B1.** `preview_token` as defined cannot cover confirm tools without a draft entry (close, reconcile, dispose, runs, the create tools). Fix: two shapes, `Draft{…}` and `Operation{tool, canonical_args, journal_head_seq, settings_seq}`; a head advance refuses `Operation` tokens; state plainly it protects against drift only.
- **B3.** The Lean post step must carry the reversal preconditions or `reversal_reverses_posted` is provable for a model admitting A2's counterexample.
- **C1 (also Z-4).** `ledger-view`/`BooksView` duplicates `PackageBooks`; a crate depending only on ledger-core cannot implement the trait for Store/Snapshot. Fix: grow `PackageBooks`; attention and `next_steps` beside the package builder.
- **C2.** Jobs are not lease holders: the books can close under a running audit or backfill. Fix: a job holds a lease of kind `job`; close waits or cancels with a recorded reason.
- **C3.** `books.audit` in `ledger-core` cannot do integrity/attachment/backup checks (no I/O); "an `audit.run` at the current head" is unsatisfiable once the audit's own event advances the head. Fix: split core statements audit / store integrity (reuse `verify`) / server composition; define "current head" as no figure-bearing event after `through_seq`.
- **C4 (also Z-5).** P6's seccomp/Landlock extractor and `inline_base64_requires_sha` contradict D-32 and D-46. Fix: crash isolation only (separate process, rlimits, timeout), named as such; one extractor behind both `text_of` callers.
- **D1.** No alternatives weighed for P4, P4b.3, P5, P6, P8 (derived vs stored figures; HMAC token vs server-recorded last preview; new crate vs `PackageBooks`; subprocess vs `catch_unwind`; direct write-off vs allowance). P4, P5, P6, P8 have no risks list.
- Nits: A12 "or a later control role" would break payroll if read literally (code restricts to receivables/payables); A13 `max_discount`, `q×u`, `exact` are money arithmetic outside the kernel; A14 the previewed number can differ from the confirmed one (token must cover next number and head); A15 the B-10 exception (reversing the reversal of a legacy hand control line is refused) is in rules.md B-38 item 3 but not the plan; B4 `digest since_seq` needs `entry_meta` replay P4 omits; C5 `consolidate_duplicates` may withdraw an entry's only evidence; C6 extraction records tie the chain to extractor version.

### Structural (ST)
- **F6.** Four conventions for superseded text and two stacked precedence rules (5, 997). Live scope still describes removed components with no marker (851, 1759-1760, 1788, 1808, 1817, 1686). Fix: one rule in a "How to read this document" section: strike-through + "→ D-n"; fixed-heading `**As built (date, commit):**` blocks; top decisions become rows D-1..D-4, D-6, D-8, D-14.
- **F7.** P4 placed before P4b though P4b built first and P4 depends on it. Keep labels; order §8 by status then merge order; status table at the head.
- **F8.** Decisions not discoverable from phases: no Status/Affects columns; phases rely on unnamed decisions (list per phase in the report). Fix: columns; "Decisions in force" per unit; reverse index 10.4.
- **F9.** 10.3 is answered history titled as open questions. Fix: retitle closed; Q-1..Q-3 become D-3/D-4/D-2 rows; crumbs checklist into P2's as-built.
- **F10.** Missing: reading guide/glossary (S:, I-n, B-n, D-n, Q-n, R-n, T1/T2, gap n, slice n, defect n), defect index 1-24 with fixing phase, P2 slice index, dependency graph, ownership map, merge order.
- **F11.** §8 preamble (771) still mandates mutation testing (D-33). Fix: "records in docs/testing.md what P12 must mutate".
- **F12.** Risks split; §9 mixes retired risks (2, 4, 5, 13). Fix: per-unit Risks; §9 cross-cutting only; retire closed ones.
- **F14.** Cross-reference errors: 1147 "(P4b.10)"; 897 defect list omits 23/24; 53 and 1785 stale counts; 1899 "P1 says so" (3.9 does); 1007 the ask placed after the answers.
- Nits F15 (separators, heading depth, "the the first company"), F16 (no Goals/Non-goals heading).

### Style (SY)
- **M1-M4.** "draft" (P4 draft entry vs preview PDF), "document" (four senses), "slice" (P2 slice n vs P4b Sn), "record" used ambiguously. Fix: "preview" for invoice previews; define document = invoice or bill; always qualify slices.
- **M5.** Built/unbuilt/stale text unmarked across the range (list in report). Fix: status tag per bullet; imperative for unbuilt.
- **M6.** Line numbers disagree and use "~". Fix: symbol names only.
- **M7.** Hedges to resolve: "once P4 lands" (1236, 1285); "about 1,800 characters"; "if it grows"; "if ever needed"; "or a later control role"; "About 7 new tools" (eight listed); "for now"; "if the company qualifies"; "`egui_term`, or our own"; "for a set time"; "the documented maximum"; "or both"; "perhaps"; "once it stabilizes"; "after the highest legacy id"; "for one release".
- **M8.** Owner-facing prose inside P4b.1 hides rules (aging boundary 1035, sent date 1041, UCC warning 1051, handbook list 1054, the gross-method DECISION 1042). Fix: a "Rules derived from P4b.1" list; owner quotes move to §10.
- **M9.** SHOULD/refused/NEVER inconsistent in spec blocks. **M10.** Long semicolon lists (974, 976-977, 1002, 1003, 1097-1104, 1330, 1594, 1597, 1649, 1744, 1805-1812, 1818, 1827, 1864). **M11.** No Accept per P4b slice. **M12.** Bill line `account` optional/required/"as before" stated three ways. **M13.** "Re-created on DATE" vs determinism (DATE = `as_of`). **M14.** Manual screenshot step unnamed. **M15.** Private change log and goldens paths. **M16.** Features vs upgrades count. **M17.** P10 T1/T2 stated twice, undefined. **M18.** Subjective criteria. **M19.** Unexpanded abbreviations. **M20.** Verbless scope fragments. **M21.** DECISION vs CHOICE vocabulary. **M22.** bookspec test placement.
- Nits N1-N15: tool-name formatting, four names for the control check, `reminder_days` vs `remind_days`, date/range/unit forms, pseudo-Rust blocks, `a | b` notation undefined, heading 1573 carrying a procedure, informal voice, redundancies, garbled sentences (1836, 1819), ambiguous "or" (1362), unexplained "informational"/"2100 aging".

## The per-unit template (structural reviewer; adopted for the rewrite)
Status | Depends on (merge first; owner go-ahead per D-39) | Runs in parallel with | Blocks | Decisions in force | Defects closed | Feature flag(s) — then **Build** (Core, Store/journal, MCP, Viewer, Exports/handbook, First-company steps; each bullet ends with `crate/path::symbol` new|changed), **Owns**, **Touches shared**, **Proofs**, **Tests** (every test named; bookspec ids; the docs/testing.md P12 entry), **Docs**, **Done when** (numbered, checkable by the EM), **Risks**, **Open questions for the owner**, **As built (date, commit)**.

## Proposed §8 table of contents (structural reviewer)
8.0 How work is organized (8.0.1 status and dependency table; 8.0.2 dependency graph and merge order; 8.0.3 shared files and merge order; 8.0.4 every unit's bar) · 8.1 Landed (P0, P1, P2 with slice index, P3, Projects, P4b S1-S3; as-built blocks kept) · 8.2 Remaining, in merge order (P4b S4, P4, P4b S5, S6, S7, P5, P6, P7, P8, P9, P10, P11, P12, P13; each in the template) · 8.3 P4b reference design (former P4b.1-15, 17) · 8.4 Not scheduled.
