# AGENTS.md — wanderbooks

Operating manual for agents working in this repo. Read this first, then the
docs it points to. The short version: this is an engine-first, clean-room
rebuild of canonical double-entry bookkeeping, done the storytold way —
agents verify against oracles, track parity automatically, and never ship
what they haven't demonstrated.

## What this is

`wanderbooks` is a from-scratch implementation of double-entry bookkeeping
semantics, built to become a full bookkeeping application (engine now, UI
later). The reference points:

- **Semantic oracle:** GnuCash. It is open source with open file formats
  (XML/SQLite), which makes it both the spec and the reference implementation
  we diff against. See `docs/oracle.md`.
- **Workflow reference:** QuickBooks Online and Xero — for *what the UI must
  do*, never for code. We observe behavior; we write our own implementation.
- **Method reference:** storytold/ArtCraft's app-cloning pipeline
  (screenshot-in-the-loop, render oracles, automated parity floors). We adapt
  it to accounting: semantic oracles instead of pixel oracles.

## The non-negotiables

1. **Money is `Money`.** Exact `Decimal` plus a currency code. Binary floats
   (`f32`/`f64`) never appear in money paths — not for amounts, not for
   rates, not for "temporary" calculations. If you need a ratio, it is a
   `Decimal`.
2. **Every entry balances.** Debits equal credits exactly. No epsilon.
3. **Posted is forever.** Posted entries are immutable. Corrections are
   reversing entries linked by `reversal_of`. There is no edit, no delete.
4. **Reports are pure functions of a ledger.** If a report needs state that
   isn't in the ledger, the ledger model is wrong — fix the model.
5. **Clean room.** Build from public specs and observed behavior only:
   GnuCash documentation and file formats, the IFRS conceptual framework,
   IRS publications for tax-adjacent logic later. GnuCash itself is GPL —
   read its docs, never copy its code. This repo is MIT OR Apache-2.0.
   Never disassemble or copy from proprietary software.

## Verification protocol (read this twice)

There is no UI yet, so "seeing what it's building" means **showing the
numbers**. The analog of storytold's screenshot-in-the-loop:

- A green `cargo test` is necessary but **not sufficient**. Every behavior
  change must be demonstrated through the public API with human-readable
  output: print the trial balance, the P&L, the balance sheet. Show the
  numbers moving the way accounting says they should.
- New behavior needs new tests in `crates/ledger-core/tests/invariants.rs`
  (or a new test file for a new crate). Tests encode accounting law, not
  implementation details: prefer "the accounting equation holds" over
  "this function returns that struct".
- **Golden files are sacred.** `tests/golden/*.json` hold hand-verified
  expected reports for canonical fixtures. If a change intentionally alters
  output, recompute the expected numbers *by hand from accounting principles*,
  update the golden, and explain the recomputation in the commit message.
  Never edit a golden to match output you haven't verified — that is how
  oracles die.
- The randomized invariant test (`random_balanced_journals_preserve_invariants`)
  is the always-on property oracle. Extend it when you add invariants.

## Parity tracking

`docs/parity.md` is the parity catalog: every semantic feature we intend to
clone, with status. Rules:

- Every feature change updates `docs/parity.md` in the same commit.
- The **floor never decreases**: the count of `done` items is monotonic.
  CI will enforce this once the catalog is machine-readable (roadmap phase 3);
  until then, it is enforced by review — yours.
- Statuses are `done` (implemented + tested + golden-covered), `partial`,
  `todo`. No other statuses. `done` requires all three.

## Engine-first architecture

- All mutations are commands on `Ledger` (`create_entry`, `add_line`,
  `post_entry`, `reverse_entry`, `add_account`). Struct fields stay private;
  no other mutation path may exist.
- `ledger-core` depends only on `rust_decimal`, `thiserror`, `chrono`,
  `serde`, `serde_json`. New dependencies need justification in the commit
  message. No I/O, no networking, no time-source in the core.
- Future crates (`ledger-store`, `ledger-import`, `ledger-oracle`, `books-app`)
  depend inward toward the core, never the reverse. See `docs/architecture.md`.

## Working here

- Rust stable, edition 2024. `cargo test` must pass; `cargo clippy` should be
  clean; `cargo fmt` before committing.
- Conventional commits: `feat:`, `fix:`, `docs:`, `test:`, `refactor:`.
  One logical change per commit. The commit message explains the *accounting*
  reasoning, not just the code.
- You have a very large context window. Use it: when working on semantics,
  load the whole `ledger-core` crate plus `docs/requirements.md` rather than
  guessing from snippets.
- If a requirement in `docs/requirements.md` conflicts with code, the
  requirement wins — then fix the code, and say so.

## Docs index

| Read this | When you need |
|---|---|
| `docs/requirements.md` | What the engine must do (R-1…), what is future work |
| `docs/architecture.md` | Crate layout, command pattern, dependency rules |
| `docs/oracle.md` | How we prove correctness: golden files, property tests, GnuCash diffing |
| `docs/parity.md` | Feature catalog and the non-regression floor |
| `docs/glossary.md` | Debit/credit and friends, for agents new to accounting |
| `plan/STATUS.md` | Where the project stands right now |
| `plan/roadmap.md` | Phase plan: semantics → oracle harness → egui app → workflow parity |

## License

MIT OR Apache-2.0. New files carry no license headers; the license files at
the repo root cover the tree.
