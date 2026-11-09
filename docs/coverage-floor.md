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

**Floor: `ledger-core` 90.01% lines, `ledger-store` 91.71% lines** (2026-11-08: the first measurement, from the full `just coverage` run; `ledger-core` 16,839 of 18,708 lines, `ledger-store` 14,255 of 15,543; 256-case properties everywhere, no special shard settings. The store measured 91.98% on an earlier run the same day; properties choose random cases, so a few tenths move between runs, which the 0.5 pp tolerance absorbs. The floor records the lower run.)

| Crate | Lines | Regions | Branches | Lines (covered / total) | Measured |
|---|---|---|---|---|---|
| `ledger-core` | 90.01% | 88.86% | 82.65% | 16,839 / 18,708 | 2026-11-08 |
| `ledger-store` | 91.71% | 85.27% | 80.73% | 14,255 / 15,543 | 2026-11-08 |

## First measurement: the fifteen files with the most uncovered lines

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
