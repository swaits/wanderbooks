# Testing strategy

How wanderbooks knows its books are right. Strongest oracle first: a
test's worth is where its idea of "correct" comes from, not what it runs.
Formal proofs are in `docs/verification.md`; the authorities every rule
comes from are in `docs/spec/`. Scope so far: `ledger-core` in full, and
the store's journal safety; the other crates come next (gap list).

## Layers

| Layer | Oracle (rank) | Where | Command | Runtime | Gates |
|---|---|---|---|---|---|
| Spec-derived examples | written specs with sources (1) | `tests/spec_*.rs`, `invariants.rs`, `statements.rs`, `assets.rs`, ... | `just check` | seconds | every change |
| Regressions | the bug, named (1) | beside the code they cover, `*_regression`/named for the defect | `just check` | seconds | every change |
| Compatibility contracts | journals written under older rules must open (1) | `ledger-store/tests/store.rs` (`books_written_under_older_rules_still_open`) | `just check` | <1s | every change |
| Properties | laws over all inputs (2) | `properties.rs`, `spec_*.rs`, `kernel_bridge.rs` | `just check` | seconds | every change |
| Metamorphic | scaling, reversal, ordering invariance (2) | `spec_reports.rs` | `just check` | seconds | every change |
| Refusal law (store) | a refused command never reaches the journal; the books reopen to the same reports (2, 3) | `store.rs` `refused_commands_never_reach_the_journal` | `just check` (`STORE_CASES=5000` deeper) | ~20s | every change |
| Fault injection (store) | a full disk or a cut-off append leaves whole, openable books (1) | `ledger-store/tests/faults.rs`, `store.rs` (`power_loss_at_any_byte_of_the_last_append_is_recovered`) | `just check` | ~5s | every change |
| Model-based stateful | the Lean state machine, transcribed (3) | `tests/model.rs` | `just check` | ~3s | every change |
| Differential: Decimal vs i64 | the kernel at two types (3) | `kernel_bridge.rs` | `just check` | <1s | every change |
| Differential: GnuCash | another implementation, adjudicated (3) | `tests/oracle_gnucash.rs`, `oracle/gnucash/` | `just oracle` | ~2 min | weekly |
| Formal proofs | Kani (bounded) and Lean (unbounded) | `src/proofs.rs`, `verify/lean/` | `just prove` | ~3 min | weekly |
| Fuzzing | crash, hang, broken books (4) | `fuzz/fuzz_targets/`, corpora in `fuzz/corpus/` | `just fuzz <target> <secs>` | 10 min each | weekly |
| Miri | undefined behavior in our use of dependencies (4) | the core's unit, bridge, invariant tests | `just miri` | ~2 min | on demand |
| Golden | `tests/golden/fixture_company.json` (5) | `invariants.rs` | `just check` | <1s | every change |
| Benchmarks | a baseline (6) | `benches/books.rs` | `just bench` | ~1 min | none (no budget, by decision) |
| Mutation | can the tests fail? | `ledger-core` | `just mutants` | ~12 min | weekly |

CI: `.github/workflows/check.yml` runs `just check` on every push;
`scheduled.yml` runs proofs, the oracle, fuzzing, and mutation weekly.

Seeds and failure databases: proptest's `*.proptest-regressions` files
are checked in and replay on every run. The GnuCash oracle's books come
from fixed seeds (`SEED` in `oracle_gnucash.rs`). Fuzz corpora live in
`fuzz/corpus/<target>`.

Golden review: `fixture_company.json` is hand-verified (docs/oracle.md,
Layer 1). The van depreciation golden was replaced on 2026-10-13 by a
hand computation (`van_schedule_is_the_hand_computed_one`) after every
row was checked against it.

## Benchmarks (baseline 2026-10-13)

No budget, by decision (Stephen, 2026-10-13: the books do little work
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

## Assumed, not confirmed (reject any)

Each was inferred, not decided by a human; the tests that rely on them
say so in their doc comments.

- ASSUMED: a P&L for a period whose start is after its end is refused.
- ASSUMED: a balance sheet omits accounts with a zero balance.
- ASSUMED: the lifetime `profit_and_loss` counts closing entries
  (only period statements exclude them).
- ASSUMED: closing a period with nothing to close still marks it closed.
- ASSUMED: a sweep account (draws, distributions) is zeroed as of the
  period end, like income.
- ASSUMED: reconciliation balances are debits minus credits (tested on
  cash; liability accounts unspecified).
- ASSUMED: a members' equity statement allocates prior unclosed earnings
  and the period's income separately; one statement's ending for a member
  may differ by a cent from the next statement's beginning (only totals
  are guaranteed to chain).
- ASSUMED: cash flow lists one working-capital or financing line per
  account that changed; with no registered assets, investing is zero.
- ASSUMED: an asset's life ends on purchase date + life months, that day
  not held.
- ASSUMED: a payroll entry has one line per account total, not per
  employee.
- ASSUMED: a period with exactly the dates of an existing one nests in it
  (is allowed).

## Gap list: what isn't covered, and what would bring it in

| Technique | Status | Trigger |
|---|---|---|
| Fault injection | SKIP for the core (no I/O, clock, or network); DONE for the store's journal (full disk, cut-off append) | the first I/O or clock in `ledger-core` |
| Integration | SKIP for the core (no collaborators) | the core gains one; covered at the store and MCP layers |
| End-to-end | SKIP for the core (a library) | MCP stdio tests cover the journeys |
| Security properties | SKIP for the core (no roles, no secrets) | the first permission concept, or a secret-bearing field |
| Concurrency stress | SKIP for the core (no shared state) | the first `Arc`, atomic, or lock in the core; the GUI and MCP server have threads (next) |
| Lean: apportion's sum law for any n | OPEN | Kani covers n <= 3, properties n <= 5; needs a permutation lemma |
| ASC paragraphs not yet read | OPEN | ASC 360-10-35-4, 340-10, 230-10-45-10, 606-10-55-37B, 210: needs a FASB basic-view login |
| GnuCash's own close | OPEN | the oracle books closes as ordinary entries; comparing GnuCash's Close Book needs its Scheme API |
| Store: differential replay vs. projection, concurrency (writer + snapshot reader), benchmarks at 10k, mutation | NEXT | then import, MCP, config, GUI, the same playbook |
