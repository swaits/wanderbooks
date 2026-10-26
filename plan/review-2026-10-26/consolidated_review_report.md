# Consolidated review of `plan/served.md` (P4b S4 through P13), 2026-10-26

Status: resolved (the fixes below were applied to `plan/served.md` on 2026-10-26; see "What changed").

Nine-stage Elephant-Goldfish review: Stage 5 zoom-out (two reviewers), Stage 6 comprehension, Stage 7 design/structural/style, Stage 8 intermediate zoom-out on the proposed mitigations, Stage 9 implementation readiness and codebase details. Stage reports in this folder. Reviewer id prefixes: Z (zoom-out), C (comprehension; letter.number), DC (design), ST (structural), SY (style), IZ (intermediate zoom-out), IR (readiness), CD (codebase details).

## Verdict
The document was sound as a design record but not executable as a hand-off: no remaining unit could be built from the text alone (C, all units), two phases contradicted the owner's recorded decisions (P7 vs D-28/D-29; P6/P9/P11 vs D-31/D-32/D-46), P4 was never reconciled with the P4b slices built after it, every code pointer was stale, and nothing stated status, dependencies, file ownership or done criteria. The accounting design of P4b is right in the code and wrong in one sentence of the plan (DC-A1). The remaining design carried speculative scope the owner never asked for (IZ, Z).

## Action items, by priority, with resolution

### 🔴 Blockers
| # | Action | Findings | Resolution |
|---|---|---|---|
| B1 | Add a status / dependency / shared-files / merge-order section at the head of §8, and one template for every remaining unit | Z-1, ST-F1, ST-F3, ST-F7, C-A2, C-A6, IR-CC-1, IR-CC-3, IZ-2 | PROPOSE FIX: new §8.0 (status table, dependency graph and merge order, shared files and owners, every unit's bar, recipes R-A..R-G, the journal-safety checklist); 8-field template (IZ-2's cut of ST's 17) |
| B2 | P7 rewritten: the vault stays v1; the scanner defect is the only code; vault v2 and TIN pages closed by decision | Z-2, ST-F2, C-I1..I4, SY (P7), IZ-3a | PROPOSE FIX: D-48; strike-throughs in 2.1, 3.4, 3.5, 4.1, 4.9, 5.2, 5.6, §6, 7.6, I-7, D-16, D-19 |
| B3 | Drop "reversal as a draft"; P4 becomes a small sweep after P4b; `void_document` stays immediate | IZ-0, DC-A2, DC-A3, DC-B3, ST-F4, Z-3, C-F1..F3, C-E5 | PROPOSE FIX: D-47; I-4 and I-5 reworded; P4 rewritten; 4.9/5.2 rows struck |
| B4 | Correct `outstanding(d, D)` to the date-sum (the shipped code and rules.md B-38 item 2) | DC-A1 | PROPOSE FIX in P4b.3 and P4b.13 |
| B5 | Number the owner's answers A-1..A-14 and fix every "decision N"; delete the override sentence; reconcile the four known conflicts (payment methods list; exempt keys; `Sent` with sender and reference; the fee) | SY-B1, SY-B2, ST-F13, C-B1, C-C1, C-D1, Z-14 | PROPOSE FIX |
| B6 | One remaining P4b feature (`documents_v2`) for S4-S7, P4's `drafts_v2` in the same upgrade; the refusal window stated once | IZ-4a, ST-F5, C-B11, SY-M16 | PROPOSE FIX; Q-5 asks the owner whether to upgrade earlier |
| B7 | S4: the issued PDF is filed as `receipt.attached` on the entry plus `attachment.linked` to `Subject::Invoice`; no `issued_pdf` field; render before journaling | CD-A6, C-B6, IZ-4d | PROPOSE FIX |
| B8 | P6: no sandboxed `ledger-extract`, no lopdf, no migration tools, no `attach_email`; extraction in `ledger-import` behind both callers; keep D-20's categories | Z-5, Z-6, DC-C4, C-H1, C-H2, CD-E1, CD-E3, IZ-3b | PROPOSE FIX; CD-E3 corrects earlier stages: the only PDF text reader is a test helper, so P6 promotes it |
| B9 | P9 split (P9a recipe + bearer now; P9b claude.ai via external IdP later; P9c not planned); P10 restated against what exists; P11 split and the helper references removed | Z-8, Z-10, Z-12, ST-F8, C-K1..K5, C-L1..L4, C-M1..M5, SY-B5, SY-B6, IZ-3c, IZ-3e, CD-G1..G5 | PROPOSE FIX; D-51 (P9c), Q-4 (P11 manifest), Q-6 (D-7) |
| B10 | P5: prompts capability does not exist yet; the envelope is a single-owner step; no `books.job`; `books.start` v2 extends the built structs | CD-D1..D3, IR-P5-1, Z-9, C-G1..G8 | PROPOSE FIX |
| B11 | P8: compose from existing checks; a `Finding` type; one `policy.attestation_bar` setting; one `audit.run` event; `workpaper` category; a check table | Z-7, DC-C3, C-J1..J5, IZ-3d, CD-F1..F3, IR-P8-1 | PROPOSE FIX |
| B12 | P13 starts from the in-repo seed and loader; `Engine` trait signature; in-tree until the harness is stable | Z-17, C-O1..O4, CD-I1, CD-I2, IR-P13-1 | PROPOSE FIX |
| B13 | Pre-step units with a single owner: `server.rs` split by tool group; attention engine over `PackageBooks` (no `ledger-view`); `docs/testing.md` sections for S1-B, S2, S3 | Z-4, Z-13, DC-C1, C-A5, IR-CC-2, IZ-4e | PROPOSE FIX |

### 🟡 Major
| # | Action | Findings | Resolution |
|---|---|---|---|
| M1 | S6: the fee by the payout pattern with one kernel identity check (`kernel::settle`) and Kani; line indices on the existing payment events; `kernel::discount_max`; bridge properties | DC-A5, DC-A4, DC-A13, IR-S6-3, IZ-1a, IZ-1e | PROPOSE FIX (CONTEST DC-A4's new event type and DC-A5's `payout_plan` refactor, per IZ) |
| M2 | S7 cut to `write_off_invoice` + `bad_debts` + B-41 + Lean `Step.writeOff`; `adopt_control_line`, `move_balance`, `owed_on`, `Movement::Recovery`, `excess_to`, apportioned discounts recorded as deferrals with handbook procedures | DC-A6..A9, DC-A11, C-E1..E3, IZ-1b..1g | PROPOSE FIX: D-49 (CONTEST the mechanisms; adopt the procedures) |
| M3 | Evidence gate: policy evaluation moves to `ledger-core`, `Store::post_entry` calls it, the GUI's string heuristic goes; at-once tools stay as today | DC-A10, DC-B2, C-F4, CD-C3, IZ-1h | PROPOSE FIX |
| M4 | Preview check: one `preview` digest at the result envelope, required by every confirm tool; no HMAC key | DC-B1, DC-A14, Z-19, C-F2, IZ-1i | PROPOSE FIX: D-50 amends I-6 |
| M5 | Strike-throughs with "→ D-n" for every superseded sentence in §§2-7, 9; one "How to read this document" section; §10.3 closed; new decision rows; 10.4 decisions by unit | Z-15, ST-F6, ST-F8, ST-F9, ST-F10, ST-F12, SY-M5 | PROPOSE FIX |
| M6 | Symbol names, never line numbers; stale counts and references fixed (defects 23, 24; schema 15; server.rs 12,566 lines; "22 reproductions") | Z-11, C-A3, SY-M6, ST-F14 | PROPOSE FIX |
| M7 | S4 contract gaps: `due_date` fallback order, `terms` type, numbering width and `first_number` default, q×u rule, `vendor_ref` on legacy bills, `bill_to` type, `default_terms` per side, handbook id, `Snapshot` → `PartySnapshot`, `company.email`/`company.website`, `PartyField` replay fix | C-B2..B13, SY-B3, CD-A3..A9, CD-A11 | PROPOSE FIX |
| M8 | S5 defaults and predicates; `Sent.by`, `Sent.reference` | C-C1..C5, IR-S5-2 | PROPOSE FIX |
| M9 | §8 preamble per D-33; per-unit "For P12" obligation | ST-F11, Z-20, C-N1..N4, CD-H1 | PROPOSE FIX |
| M10 | Style: "preview" vs "draft", "document" defined, slices qualified, hedges resolved, run-on lists split, `the the first company` fixed, `<Company>` placeholder | SY-M1..M22, SY-B4 | PROPOSE FIX within §8; the rest of the file keeps its prose (CONTEST the full rewrite of §§1-7 as out of scope) |

### 🟢 Nits
Tool-name formatting, `remind_days` naming, unit forms, `a | b` notation defined, heading 1573's procedure moved under the heading, verbless fragments, `DECISION` vs `CHOICE` mapping (SY-N1..N15, SY-M21): applied where §8 was rewritten; not chased through §§1-7.

## Contested (not applied)
- DC-A4 `settlement.posted` event; DC-A5 `payout_plan` refactor; DC-A8 adopt variants; DC-A9 `owed_on`; DC-A11 `Movement::Recovery`; DC-A6 `excess_to`; DC-A7 apportioned discounts; DC-B1 two-shape HMAC; DC-C2 jobs as leases: each replaced by the simpler resolution in the Stage 8 report (IZ-1a..1k) and recorded as a deferral (D-49) or a procedure.
- ST's 17-field template: 8 fields (IZ-2).
- Z-10's "minisign only": conflicts with D-25/D-26 as recorded; asked as Q-4 instead.
- SY-M8 moving P4b.1 out of the plan: kept (the handbook draws on it); the derived rules are listed beside it.
- IR's P4 step 3 (reversal as draft): superseded by IZ-0.

## Traceability
Every finding id above maps to one action item; findings not listed (SY nits in §§1-7, ST-F15/F16 formatting) were judged below the bar for this pass and are recorded here for a later editing pass.

## What changed in `plan/served.md`
See the top of the file ("0. How to read this document", "What changed on 2026-10-26") and §8.0.
