# P14 — bookspec, the definitive spec; the kernel sweep; verification, more

The owner's goal (2026-11-08): bookspec (`git.swaits.com/swaits/bookspec`)
becomes THE specification for accounting and ledgers, usable directly by
anyone building an engine, and wanderbooks' code sources from it. Alongside,
every money computation moves into the proved kernel, the Lean model is
checked against the code by machine, and a second outside oracle joins
GnuCash. Decisions D-68 to D-72 (section 10.1 of `served.md`). Inputs:
`plan/bookspec-audit.md` (the spec and Lean move, section 6 has the unit
details), `plan/kernel-sweep-audit.md` (every money site outside the
kernel, panics, the sweep units in section 3, hledger in section 4).

## How bookspec is shaped (D-68)

- **One structured record per rule** (TOML, `deny_unknown_fields`, checked
  by bookspec's checker): `id` (`BS-<area>-<n>`, never reused), `title`,
  `description` (plain English, required: the human-readable part),
  `statement` (numbered MUST / MUST NOT sentences), `status` (AUTHORITY,
  CONVENTION, CHOICE), `options` (CHOICE only: named options, each with
  provenance), `sources` (exact citations pinned into the vendored text),
  `invariants` (each naming the Lean theorem that proves it, or `none:`
  with the reason), `scenarios`, `examples` (ids), `class`, `profile`,
  `retired`. Prose files stay only where structure can't carry it (the
  model, how to read the spec, how to implement an engine).
- **The checker enforces the links**: every cited source file and pin
  exists verbatim; every named Lean theorem is in `Audit.lean`; every rule
  has an example (or is marked `procedural` / `retired`); every example
  names a rule that exists; CHOICE rules have one example per option.
- **The laws are proved once, in bookspec's Lean** (D-69). wanderbooks'
  `verify/lean` requires bookspec's lake package at the pinned rev and keeps
  only its model of its own commands.
- **Product rules** (D-71): bookspec states only the general principle;
  wholly product ids are retired; wanderbooks keeps `WB-n` rules and its
  claims (`docs/spec/claims/<area>.md`: per BS id, the option picked,
  Enforced, Checked). Product rules are numbered `WB-<area>-<n>` (the EM,
  2026-11-08: areas are moved in parallel, so one global WB-n numbering
  would collide; the shape mirrors bookspec's ids), kept in the claims file
  of the area their rule came from.

## Working rules

- bookspec work happens in worktrees of the local clone `~/Code/bookspec`
  (`git -C ~/Code/bookspec worktree add ~/Code/bookspec-wt/<unit> -b <unit>`);
  the EM merges into `~/Code/bookspec` main and pushes (D-70). wanderbooks
  work uses T3 worktrees as before.
- Each bookspec batch ends with one wanderbooks pin bump (both Cargo.toml
  revs, and the lake manifest once F2 lands), compile + touched tests per
  merge, the full gates once per batch (D-60).
- Kernel units: workers write `docs/proofs/<unit>.md` (harnesses, laws,
  bounds, times); the EM folds them into `docs/verification.md` and the
  `Audit.lean` list in one edit per batch.
- Every unit gets a review from a different model before it merges.

## Units

Sizes: S about a day, M 2-4, L 5-8 days of one worker (the audits' scale;
workers have been faster).

| Unit | What | Needs | Owns | Size |
|---|---|---|---|---|
| **S-A** | Rule records and one catalogue: the record schema above, the checker's rule loader and link checks, `rules-index.md` and `rules.rs`'s table generated from the records; B-39/40/43 statuses fixed; the page budget dropped or raised | D-68, D-71 | bookspec `checker/src/rules.rs` (+ new `rule.rs`), `checker/tests/{spec,rules}.rs`, `spec/00-model.md`, `spec/rules-index.md`, new `rules/` | M |
| **S-B** | Sources move: the 19 qualifying sources to `bookspec/sources/` with licence lines; the verbatim pin check moves into bookspec; wanderbooks keeps its 8 product sources | S-A's pin field, X-H0 | bookspec `sources/**`, `README.md`, `checker/tests/spec.rs`; wanderbooks `docs/spec/sources/**`, `docs/spec/README.md`, one adapter test | M |
| **S-C1..C6** | Spec content per area, as rule records, each unit also reducing its area's wanderbooks claims file (unit S-D for that area): C1 entry + ledger, C2 statement + period + prepaid + revenue, C3 partner, C4 asset + 1099 + tax categories, C5 arap (+ B-43), C6 formats and the retired ids | S-A, S-B | one area's records each | M-L each |
| **S-C7** | Embedded CHOICEs become their own records (the EM, 2026-11-08: one status per record, so a CHOICE inside an AUTHORITY or CONVENTION rule, such as B-12's period nesting, B-14 rule 11's presentation, B-30's default dates, B-33's 1099 counting and B-32's warn or refuse, gets its own `BS-<area>-<n>` with its options, one example per option and wanderbooks' `Pick:`; the parent keeps its authority text and points at it; content units record such CHOICEs in `status_note` until then) | S-C1..C6 | the affected records, `rules/ids-issued.txt`, the claims files | M |
| **S-D0** | Split `docs/spec/rules.md` mechanically into `docs/spec/claims/<area>.md` and `docs/spec/product.md` (WB-n), no content change | X-H0 | `docs/spec/**`, the adapter's path | S |
| **S-D** | Reduce each claims file as its C unit lands: keep the pick, Enforced, Checked, the WB rules | S-D0, S-C* | `docs/spec/claims/*.md` | S per area |
| **S-E** | Rewrite every `rules.md B-n` mention (784 in 160 files) to `BS-...` / `WB-n`; AGENTS.md points at bookspec; a test that every `SOURCE:` resolves. Single-owner moment: no other branch open | S-A, S-D0 | 160 files | M |
| **S-K1** | bookspec self-contained: the GnuCash oracle's Dockerfile and `books.py` move in, its README rewritten, Lean in its CI | none | bookspec `gnucash/**`, `oracle/**`, `README.md`, CI; wanderbooks `oracle/`, `justfile` | S-M |
| **L-F1** | Every law into bookspec's Lean: Apportion, Parties, Prorate, Deductible, Documents, `discard` and its theorems; neutral docstrings; `Audit.lean` with all 118; `spec/laws.md` | none | bookspec `lean/**`, `checker/tests/lean.rs`, new `spec/laws.md` | M |
| **L-F2** | wanderbooks requires bookspec's Lean: `[[require]]` at the pinned rev, the duplicated files deleted, `just prove` and CI updated, a test that the lake rev equals the Cargo rev | L-F1 | `verify/lean/**`, `justfile`, `docs/verification.md`, workflows | S-M |
| **L-G1** | A computable `step` for `Ledger.Step`, proved sound and complete; the four model gaps fixed (reverse-once, the reversal's date, an empty close, period nesting) | L-F1 | bookspec `lean/Bookspec/Exec/Step.lean` | M |
| **L-G3** | `lake exe bookspec-check`: the Lean model runs bookspec's examples itself (Lake's TOML parser, four queries, a skip report), so the spec checks its own examples | L-G1 | bookspec `lean/Exec.lean`, `lakefile.toml`, `checker/tests/lean.rs` | M |
| **L-G5** | Model vs code by machine: random command sequences through the Rust `Ledger` and the Lean executable, compared state by state (task B) | L-G1 | wanderbooks `crates/ledger-core/tests/lean_model.rs`, a just recipe | M |
| **L-G2, G4** | Documents in the executable model; more queries (closes to members, proration, cash flow last) | L-G3 | bookspec `lean/Exec/**` | L |
| **X-H0** | Split the 4,647-line adapter `crates/ledger-core/tests/bookspec.rs` into modules, no behaviour change | none | `crates/ledger-core/tests/bookspec/**` | S-M |
| **X-H1** | Schema additions for the missing examples (balance sheet, general ledger, tax summary, 1099 worksheet, aging, payments, drafts and evidence, refused input); and examples for every CHOICE record on `rules/allow-no-example.txt` (the EM, 2026-11-08, revised: every embedded CHOICE becomes its own record now so an engine can declare a pick for it; a record's examples must genuinely exercise it, an example is never linked to a choice it doesn't depend on, so the CHOICEs no example can show yet go on the allow-list, whose cap is raised once for them, from 9, and only shrinks after; X-H1/X-H2 work it down. Among them, from S-C7b's `s-c7b-noexample`: partner prepaid-contract, shares-in-force, prepaid basis; asset §704(c) method; 1099 payee's share; arap due-today, as-of reading, payment before its document, vendor reference compare, recovery form, gross discount, gross fee, discount apportioning; format closing entries, the customer/vendor NAME, the two-A/R refusal; and from S-C7a the records no example exercises: BS-entry-6..8, BS-ledger-14, -17, -18, -19, -21, BS-period-4, BS-prepaid-4, subject to its review) | S-A, X-H0 | bookspec `checker/src/schema.rs`; the adapter | L |
| **X-H2** | Examples from the 2026-11-07 goldens, each from its worksheet | X-H1 (4 need nothing) | bookspec `examples/*.toml` | M |
| **X-I** | Per-rule coverage in the runner with a floor (stated rules with an example never decrease); a CI step, like `scripts/parity-floor.sh`, that fails when `rules/ids-issued.txt` loses a line or the allow-list grows against the base commit (the in-tree checks can't see a record and its ids-issued line deleted together) | S-A, X-H2 | bookspec `runner/`, `checker/tests/coverage.rs` | S-M |
| **X-J** | wanderbooks traceability gate: every claimed BS id has an enforcement site and a test, every law a Lean theorem, every CHOICE a declared pick, every kernel function a Kani harness, a bridge property and a law | S-D, S-E, L-F2 | `crates/ledger-core/tests/traceability.rs`, `docs/spec/claims/**` | M |
| **X-K2** | Profiles (core, documents, partners, us-tax, formats) and an engine builder's guide; the hard-coded accounts become setup keys | S-A, X-H1 | bookspec `checker/src/{engine,schema}.rs`, `spec/00-model.md` | M |
| **X-O** | hledger as a second oracle: a pinned container (official static 1.52.4, sha256 in the Dockerfile), a bookspec engine adapter, and wanderbooks' random-book differential beside GnuCash's (task C; `plan/kernel-sweep-audit.md` section 4) | S-K1 | bookspec `hledger/**`; wanderbooks `tests/oracle_hledger.rs`, `justfile` | M |
| **K-U0** | Kernel and proofs into modules (`kernel/`, `proofs/`, `tests/kernel_bridge/`), `_kani` updated, `just prove` unchanged | none | `crates/ledger-core/src/{kernel,proofs}/**`, `tests/kernel_bridge/**`, `justfile` | S |
| **K-U1..U9** | The sweep (`plan/kernel-sweep-audit.md` section 3), in value order: U1 FX, U3 shares (six copies to one; legacy replay policies only named, never changed), U2 rounding, U5 schedules (D-72), U4 totals (the raw-operator panic class), U6 ratios, U7 payroll, U8 control and aging, U9 display rounding. Each: kernel functions, Kani, bridge, its law in bookspec's Lean, its rule cited by BS id | K-U0; Lean laws after L-F2 | its own `kernel/<unit>.rs`, `proofs/<unit>.rs`, bridge module, call sites | S-M each |
| **S-E2** | The 13 rule citations S-E kept in program output move to BS ids together: the eight B-43 evidence-source strings in `ledger.rs`, the two lines recording them in `ledger-store/tests/golden/draft_states_and_evidence_gate.json` (a string-only golden edit, verified by hand and explained in the commit), `ledger-package/src/package.rs` (CPA package text), `ledger-mcp/src/tools/settlements.rs` (a tool description), and the GUI `CONTRIBUTION_NOTE` (`rules.md B-37 item 7`). First establish whether any of these strings is stored in a journal or the store (then old books must still read: replay keeps the old text, new writes the new); then shrink `source_refs.rs`'s kept-string allow-list to zero (S-E review, 2026-11-08) | S-E merged | those six files, the golden, `tests/bookspec/source_refs.rs` | S |
| **K-U10** | `impl Amount for Decimal`'s `checked_add` and `checked_mul` exact or None (BS-ledger-1: a product the arithmetic can't hold MUST be refused; BS-entry-4): `checked_mul` rounds a product whose mantissa passes 96 bits (K-U1's fix, 2026-11-08: `1050000010.0000000001 × 9.9999999999` books 10500000099.90 for an exact 10500000099.8949…, both inputs within D-75's ten places; pinned by `the_mantissa_edge_is_outside_the_precondition_and_pinned`), and `checked_add`: `rust_decimal` rounds dust away within one unit of `Decimal::MAX` (`MAX + 0.4` gives `Some(MAX)`), breaking the kernel's exact-or-None contract (found by K-U4, 2026-11-08; pinned by the canary `finding_decimal_add_rounds_dust_at_the_edge`): refuse a sum whose result scale fell below the operands'; after the sweep units merge (the file is shared) | K-U4 merged | `kernel/entries.rs`, the canary, the bridge | S |
| **V-D** | Coverage report and floor; `rust_decimal` checked against exact rationals (task D) | none | in flight 2026-11-08 | S |
| **V-D2** | Tests where coverage is thinnest (the V-D findings, `docs/coverage-floor.md`): `ledger-store` `reconcilable.rs` (0% from the store's own suite), `ledger-core` `evidence.rs` (34% lines, 9% branches from the core's suite), `ledger.rs`'s command steps (`mark_sent_steps`, `settle_steps`, `run_payroll_steps`, …) and refusal arms, `conventions.rs` and `settings.rs` validation refusals, `audit.rs` branches; each test encodes the rule it covers; the floor rises with them | V-D | the crates' tests | M |

## Order

1. **Wave 1** (disjoint files): K-U0, X-H0, L-F1, S-A. (V-D and the
   `Money::checked_add` fix are already running.)
2. **Wave 2**: S-B, S-D0 (after X-H0), S-K1, L-F2 (after L-F1), K-U1 and
   K-U3 (after K-U0).
3. **Wave 3**: S-C1..C6 in parallel, L-G1, X-H1, more kernel units.
4. **Wave 4**: S-D per area, X-H2, L-G3, L-G5, X-O.
5. **S-E** alone, with no other branch open.
6. X-I, then X-J. L-G2, G4 and X-K2 as capacity allows.
