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
| **X-H1** | Schema additions for the missing examples (balance sheet, general ledger, tax summary, 1099 worksheet, aging, payments, drafts and evidence, refused input) | S-A, X-H0 | bookspec `checker/src/schema.rs`; the adapter | L |
| **X-H2** | Examples from the 2026-11-07 goldens, each from its worksheet | X-H1 (4 need nothing) | bookspec `examples/*.toml` | M |
| **X-I** | Per-rule coverage in the runner with a floor (stated rules with an example never decrease) | S-A, X-H2 | bookspec `runner/`, `checker/tests/coverage.rs` | S-M |
| **X-J** | wanderbooks traceability gate: every claimed BS id has an enforcement site and a test, every law a Lean theorem, every CHOICE a declared pick, every kernel function a Kani harness, a bridge property and a law | S-D, S-E, L-F2 | `crates/ledger-core/tests/traceability.rs`, `docs/spec/claims/**` | M |
| **X-K2** | Profiles (core, documents, partners, us-tax, formats) and an engine builder's guide; the hard-coded accounts become setup keys | S-A, X-H1 | bookspec `checker/src/{engine,schema}.rs`, `spec/00-model.md` | M |
| **X-O** | hledger as a second oracle: a pinned container (official static 1.52.4, sha256 in the Dockerfile), a bookspec engine adapter, and wanderbooks' random-book differential beside GnuCash's (task C; `plan/kernel-sweep-audit.md` section 4) | S-K1 | bookspec `hledger/**`; wanderbooks `tests/oracle_hledger.rs`, `justfile` | M |
| **K-U0** | Kernel and proofs into modules (`kernel/`, `proofs/`, `tests/kernel_bridge/`), `_kani` updated, `just prove` unchanged | none | `crates/ledger-core/src/{kernel,proofs}/**`, `tests/kernel_bridge/**`, `justfile` | S |
| **K-U1..U9** | The sweep (`plan/kernel-sweep-audit.md` section 3), in value order: U1 FX, U3 shares (six copies to one; legacy replay policies only named, never changed), U2 rounding, U5 schedules (D-72), U4 totals (the raw-operator panic class), U6 ratios, U7 payroll, U8 control and aging, U9 display rounding. Each: kernel functions, Kani, bridge, its law in bookspec's Lean, its rule cited by BS id | K-U0; Lean laws after L-F2 | its own `kernel/<unit>.rs`, `proofs/<unit>.rs`, bridge module, call sites | S-M each |
| **V-D** | Coverage report and floor; `rust_decimal` checked against exact rationals (task D) | none | in flight 2026-11-08 | S |

## Order

1. **Wave 1** (disjoint files): K-U0, X-H0, L-F1, S-A. (V-D and the
   `Money::checked_add` fix are already running.)
2. **Wave 2**: S-B, S-D0 (after X-H0), S-K1, L-F2 (after L-F1), K-U1 and
   K-U3 (after K-U0).
3. **Wave 3**: S-C1..C6 in parallel, L-G1, X-H1, more kernel units.
4. **Wave 4**: S-D per area, X-H2, L-G3, L-G5, X-O.
5. **S-E** alone, with no other branch open.
6. X-I, then X-J. L-G2, G4 and X-K2 as capacity allows.
