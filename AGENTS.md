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

- **Spec:** accounting principles, written down as proofs
  (`docs/verification.md`). That is the only authority.
- **Cross-check oracles:** GnuCash (open source, open file formats), the
  old Google Sheet, QBO/Xero exports. Useful to diff against, *never*
  trusted to be right: a disagreement is adjudicated from principles, and
  nothing may rest on another program's behavior alone. See
  `docs/oracle.md`, "Who is right".
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
6. **Accounting laws are proved, not just tested.** Arithmetic that money
   flows through lives in `kernel.rs` and carries Kani proofs; the laws
   it serves carry Lean proofs. `just prove` stays green. See
   "Formal methods" below.

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

## Formal methods: the bar (read this three times)

Tests check the cases someone thought of; proofs check all of them. The
first formal pass on this engine (2026-10, `docs/verification.md`) paid
for itself several times over, on code that already had 190+ tests,
property tests, mutation testing, and 11.5M fuzz executions:

- **Negative line amounts were accepted by the engine.** Only the MCP
  layer refused them. Any other caller could post them.
- **A large enough amount crashed the engine** (a `Decimal` overflow
  panic while summing lines) instead of returning an error.
- **Currency conversion could crash** the same way.
- **The verifier itself had a soundness bug.** Kani 0.68 mis-models
  `copy_from_slice` of symbolic length (model-checking/kani#4904): it
  reported a reversal that doesn't net to zero. We didn't wave it away.
  Replaying the counterexample natively, then shrinking it to a 15-line
  crate, proved it was the tool, not the books. Now a canary harness
  watches for the fix, and covers prove no proof is vacuous.
- **Writing the spec simplified the code.** `credit_line` and
  `closing_line` turned out to be the same function. Lean also proved
  that no draft can exist in a closed period, the invariant that lets
  the store journal a post before applying it.

So the bar, for every agent, every change:

0. **Specs come from authorities, not memory.** Every rule the engine
   enforces is in `docs/spec/rules.md` with its source: Pacioli for the
   mechanics of double entry, the IRS (vendored verbatim, public domain)
   for records, periods, 1099s, capitalization, partnerships, and payroll,
   FASB's Codification (cited, never copied) for GAAP. A rule no authority
   states is marked CONVENTION or CHOICE (bookspec's word for a
   decision), with who decided and when.
   Spec blocks in the code (```text GOAL/SOURCE/INVARIANTS/SCENARIOS)
   carry a `SOURCE:` line. Never write a spec from what the code happens
   to do, or from what you remember; find the document, or ask.

1. **Money arithmetic goes in `kernel.rs`, generic over `Amount`.** If
   you write a sum, a sign flip, a split, or a rounding plug anywhere
   else, move it into the kernel. The engine calls the kernel. It never
   gets a second copy.
2. **Every kernel function gets a Kani harness** in `src/proofs.rs`, on
   fixed arrays (no symbolic-length `Vec` growth, no `copy_from_slice`),
   with a `kani::cover!` showing the interesting case is reached.
3. **Every new law gets a Lean theorem** in `verify/lean`. Laws about
   the books over time are theorems about `Reachable` states. A new
   `Ledger` command is a new `Step` constructor, with its preservation
   proof. No `sorry`, ever; `Audit.lean` lists every headline theorem.
4. **Every kernel instantiation gets a bridge property** in
   `tests/kernel_bridge.rs`, so what Kani proved on `i64` holds on
   `Decimal`.
5. **A failing proof is a finding, not an obstacle.** Get the
   counterexample, replay it natively, and decide on evidence: bug in
   our code, bug in the spec, or bug in the tool. Never weaken an
   assertion, raise a bound, or delete a harness to make red go green
   without writing down why in `docs/verification.md`.
6. **More, not less.** When you touch an area the proofs don't cover
   yet (A/R and A/P documents, payroll, depreciation, FX, persistence),
   leave it with more formal coverage than you found it. Proptest,
   mutation testing, fuzzing, Kani, and Lean all stack. Use all of them.

`just check` and `just prove` both pass before every commit that touches
`ledger-core`.

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
  `serde`, `serde_json`, and `jiff` (the bundled time-zone database for
  `company.timezone`: "today" is computed by the caller and passed in;
  the core still reads no clock). New dependencies need justification in
  the commit message. No I/O, no networking, no time-source in the core.
- Future crates (`ledger-store`, `ledger-import`, `ledger-oracle`, `books-app`)
  depend inward toward the core, never the reverse. See `docs/architecture.md`.

## Working here

- `mise install` sets up every tool (pinned in `mise.toml`): Rust,
  just, Kani, Lean, cargo-mutants, cargo-fuzz. If a recipe needs a tool,
  it goes in `mise.toml`; nothing gets installed by hand.
- Rust stable, edition 2024. `just check` must pass: fmt, clippy with zero
  warnings, every test. `just prove` must pass for core changes.
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
| `plan/served.md` | The plan every remaining unit is built from: section 8.0 (status, order, recipes), 8.2 (the units), 10 (decisions) |
| `plan/HANDOFF.md` | The prompt for the implementing team, and how to resume the paused worktrees |
| `docs/requirements.md` | What the engine must do (R-1…), what is future work |
| `docs/architecture.md` | Crate layout, command pattern, dependency rules |
| `docs/oracle.md` | How we check correctness: golden files, property tests, fuzzing, cross-checks against other implementations (and who is right) |
| `docs/spec/` | The authorities every rule comes from (`rules.md`), with public-domain sources vendored verbatim |
| `docs/verification.md` | The formal proofs: every accounting law, its Kani harness and Lean theorem, what they found |
| `docs/parity.md` | Feature catalog and the non-regression floor |
| `docs/capability-parity.md` | Parity vs the GSheet+GDrive system + the cutover rule |
| `docs/mcp-interface.md` | The MCP tool surface: the primary interface spec |
| `docs/receipt-store.md` | Blob store + `ledger-store` SQLite persistence spec |
| `docs/testing.md` | Every test layer, each unit's test record, and what P12 must mutate |
| `docs/parties-and-attachments.md` | The party directory, roles, the TIN vault, attachments to any subject |
| `bookspec/README.md` | The formal accounting specification's examples, checker and runner (P13) |
| `deploy/README.md` | Running the books on a machine of your own (P9a) |
| `docs/glossary.md` | Debit/credit and friends, for agents new to accounting |
| `plan/STATUS.md` | Where the project stands right now |
| `plan/roadmap.md` | Where the project has been and is going: the installed app, then the hosted service |

## License

MIT OR Apache-2.0. New files carry no license headers; the license files at
the repo root cover the tree.
