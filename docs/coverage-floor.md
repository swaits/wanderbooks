# Coverage floor

Line, region and branch coverage of the two crates every other crate is
built on, measured from their own test suites. `just coverage` writes the
HTML and a per-file table for each crate under `target/coverage/<crate>/`
(`summary.txt` is the table: regions, functions, lines and branches, with
missed counts and percentages); `just coverage-check` reads that report and
fails when a crate's **total line coverage** is more than 0.5 percentage
points below its recorded floor.

Why a floor, not a target: coverage is a tripwire for silently losing what
a test used to reach. It is not a measure of correctness — the property
tests, the GnuCash oracle, mutation testing and the proofs answer that. A
change that deletes, narrows or replaces a test must not also let the
report drift down unnoticed.

- Tool: `cargo-llvm-cov` 0.9.1, pinned in `mise.toml`. Branch coverage is
  nightly-only (`-Zcoverage-options=branch`), so the run uses the pinned
  `nightly-2026-10-01` and its `llvm-tools` component, also in
  `mise.toml`.
- Measured: `ledger-core` and `ledger-store` library sources, each from
  its own suite (`cargo llvm-cov -p <crate>`). Excluded:
  `src/proofs.rs` (Kani-only, `cfg(kani)`, never in the test binary),
  `tests/`, `benches/` and `examples/`, and the other workspace crates a
  dev-dependency compiles instrumented (the core's tests use the store,
  the store's use the core).
- The floor is the total line coverage recorded on the measurement date.
  Regions and branches are recorded for context but not gated (branch
  coverage on a nightly flag is noisier across compiler versions; line
  coverage is the stable signal). 0.5 percentage points are tolerated for
  measurement movement; more means a test stopped reaching what it used
  to.

**Floor: `ledger-core` 95.49% lines, `ledger-store` 92.58% lines** (2026-11-09: `ledger-core` from the full `just coverage` run on V-D2's merged, reviewed tree, 18,422 of 19,292 lines; `ledger-store` stays at 92.58%, its V-D2 measurement, since that run measured 92.45% (14,359 of 15,531, the same lines as before: main's store changes were comments and the fixes added no store code), a few tenths of run-to-run movement in the store's property and process tests that the 0.5 pp tolerance absorbs. A floor only rises: a run inside the tolerance is recorded in the table below, never written over the floor. 256-case properties everywhere, no special shard settings. The first floors, 90.01% and 91.71% (2026-11-08), are in the history below.)

| Crate | Lines | Regions | Branches | Lines (covered / total) | Measured |
|---|---|---|---|---|---|
| `ledger-core` | 95.49% | 93.09% | 88.98% | 18,422 / 19,292 | 2026-11-09 (V-D2, reviewed) |
| `ledger-store` | 92.45% | 85.86% | 81.72% | 14,359 / 15,531 | 2026-11-09 (V-D2, reviewed) |
| `ledger-core` | 95.45% | 93.06% | 88.90% | 18,391 / 19,267 | 2026-11-09 (V-D2, before review) |
| `ledger-store` | 92.58% | 85.98% | 81.72% | 14,379 / 15,531 | 2026-11-09 (V-D2, before review) |
| `ledger-core` | 90.01% | 88.86% | 82.65% | 16,839 / 18,708 | 2026-11-08 (first) |
| `ledger-store` | 91.71% | 85.27% | 80.73% | 14,255 / 15,543 | 2026-11-08 (first) |

## Second measurement: V-D2, tests where coverage was thinnest (2026-11-09)

The first measurement named its holes; V-D2 wrote the tests that reach
them, each encoding the rule the code serves (the rule ids are in the
commits and in `docs/testing.md`, "V-D2"). The run at the start of the
unit, on the same code and tools, is the "before": the core had drifted to
89.79% (the code had grown since 2026-11-08), the store to 92.09%.

| File | Lines before | Lines after | Branches before | Branches after |
|---|---|---|---|---|
| `ledger-core/src/evidence.rs` | 33.85% (129 of 195 missed) | 100.00% (0 missed) | 8.93% | 96.43% |
| `ledger-core/src/conventions.rs` | 77.76% (145 of 652 missed) | 97.24% (18 missed) | 78.21% | 96.15% |
| `ledger-core/src/audit.rs` | 80.33% (94 of 478 missed) | 94.14% (28 missed) | 61.36% | 84.09% |
| `ledger-core/src/settings.rs` | 91.05% (117 of 1,307 missed) | 98.16% (24 missed) | 81.25% | 89.84% |
| `ledger-core/src/ledger.rs` | 86.00% (862 of 6,158 missed) | 96.87% (193 missed) | 80.38% | 91.32% |
| `ledger-store/src/reconcilable.rs` | 0.00% (76 of 76 missed) | 100.00% (0 of 78 missed) | 0.00% | 100.00% |
| `ledger-core` total | 89.79% (17,299 of 19,267) | 95.45% (18,391 of 19,267) | 82.26% | 88.90% |
| `ledger-store` total | 92.09% (14,301 of 15,529) | 92.58% (14,379 of 15,531) | 80.92% | 81.72% |

What is left in the core is mostly arms no posted book reaches: a sum past
exact arithmetic in the audit's equation, control and register checks (a
journal line is capped at 1e15, so no realistic sum overflows `Decimal`),
and the defensive arms of `ledger.rs` (about 190 lines of scattered
replay-only and overflow refusals). `reconcilable.rs` gained two lines
because the fix to the digest's account order (its own commit) added them.

## First measurement: the fifteen files with the most uncovered lines (2026-11-08)

| File | Lines | Regions | Branches | Uncovered lines |
|---|---|---|---|---|
| `crates/ledger-core/src/ledger.rs` | 86.10% | 84.69% | 80.40% | 854 |
| `crates/ledger-store/src/store.rs` | 90.39% | 78.45% | 81.93% | 641 |
| `crates/ledger-store/src/root.rs` | 86.71% | 80.21% | 73.87% | 197 |
| `crates/ledger-core/src/conventions.rs` | 78.29% | 75.95% | 78.21% | 142 |
| `crates/ledger-core/src/evidence.rs` | 33.85% | 30.05% | 8.93% | 129 |
| `crates/ledger-core/src/settings.rs` | 91.05% | 87.70% | 81.25% | 117 |
| `crates/ledger-core/src/cashflow.rs` | 91.63% | 88.55% | 87.64% | 110 |
| `crates/ledger-core/src/audit.rs` | 80.50% | 78.33% | 61.36% | 93 |
| `crates/ledger-store/src/backup.rs` | 95.16% | 94.40% | 82.88% | 83 |
| `crates/ledger-core/src/arap.rs` | 87.97% | 87.67% | 91.46% | 77 |
| `crates/ledger-store/src/reconcilable.rs` | 0.00% | 0.00% | 0.00% | 76 |
| `crates/ledger-core/src/parties.rs` | 88.37% | 86.30% | 81.40% | 67 |
| `crates/ledger-store/src/meta.rs` | 91.32% | 84.56% | 83.33% | 66 |
| `crates/ledger-store/src/describe.rs` | 81.99% | 79.89% | 76.92% | 49 |
| `crates/ledger-core/src/kernel.rs` | 95.95% | 91.34% | 88.56% | 46 |

`reconcilable.rs` is the one whole-module hole: the store exposes it,
but the server and the viewer are its readers, so the store's own suite
never calls it. `ledger.rs`'s uncovered half is mostly the replay steps
of document and payroll commands (`mark_sent_steps`, `settle_steps`,
`run_payroll_steps`, ...), which the store's replay tests exercise; the
core's own suite does not. These are findings, not tasks for this
measurement.

The doors this closes, and the ones it does not: it catches a suite that
loses its reach (a deleted test, an ignored module, a helper no longer
called) and a source file added without tests, at the granularity of total
lines per crate. It does not try to floor each file: a refactor that moves
code between files would then fail for no accounting reason. Per-file
numbers are in `target/coverage/<crate>/summary.txt`; the fifteen files
with the most uncovered lines on the first measurement are the table above.
