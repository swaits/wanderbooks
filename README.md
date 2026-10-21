# wanderbooks

Canonical double-entry bookkeeping, rebuilt from scratch. The engine is
verified against oracles, parity is tracked automatically, and the primary
interface is AI-first: you talk to an agent, the agent drives the books
through MCP. Self-hosted, no rent.

## What it is

- **`ledger-core`** — the semantics engine. Exact decimal money, five
  account types, balanced journal entries, immutable posted log with
  reversing entries, trial balance, P&L, balance sheet, the statement of
  cash flows (direct method, with the reconciliation and noncash
  schedules), fixed-asset register with straight-line depreciation.
- **`ledger-store`** — durable storage. The source of truth is an
  append-only, hash-chained JSONL journal; SQLite is a disposable
  projection rebuilt from it; receipts live in a content-addressed blob
  store. Drafts are durable, corrupt projections self-heal.
- **`ledger-mcp`** — the AI-first interface. 64 MCP tools over stdio or
  Streamable HTTP: draft → preview → post workflow, server-side receipt
  policy (>$25), anti-splitting detection, asset management, digest for
  human review, plus read-only resources for dashboards.
- **`ledger-import`** — QuickBooks interop. Import bank/credit-card CSVs
  and chart-of-accounts CSVs; export IIF journal entries (for QuickBooks
  Desktop) and XLSX reports.
- **`ledger-config`** — the company settings in a books dir's
  `wanderbooks.toml`.
- **`ledger-gui`** — the viewer: a read-only, live window on the books for owners and their
  accountant (egui). Answers first (cash, runway, profit, what needs
  attention), then every number drills down to its lines, its entry, the
  rendered receipt, and the journal history. Follows the journal live:
  a change the agent makes appears in milliseconds. `--demo` opens a
  realistic demo company.

## Verification

This is the storytold way applied to accounting: agents verify against
oracles, never ship what they haven't demonstrated.

- 190+ workspace tests: invariant tests encoding accounting law, golden-file
  report snapshots, property tests (proptest), end-to-end MCP stdio tests.
- **Formal proofs of the accounting laws** (`just prove`,
  `docs/verification.md`). Kani model-checks the engine's arithmetic
  kernel exhaustively within bounds. Lean 4 proves that, after any
  sequence of commands, the trial balance balances, the accounting
  equation holds, posted entries are forever, closed periods stay
  frozen, and a close moves exactly the net income to equity. The first
  pass found three engine bugs that every test, fuzzer, and mutant had
  missed: negative amounts accepted, and two crashes on oversized
  amounts. It also caught a soundness bug in the model checker itself.
- Mutation testing via cargo-mutants; fuzzing via cargo-fuzz (11.5M+
  executions, zero crashes).
- GnuCash is *a* behavioral cross-check, never the authority and never
  a code source (clean room, MIT OR Apache-2.0). Accounting principles,
  proved, are the spec. Where any other program disagrees, we work out
  from principles who is right.

## Status

Parity is 12 of 12 against the spreadsheet workflow it replaces
(`docs/capability-parity.md`). The parallel-run plan
(`docs/parallel-run.md`) is written; the Google Sheet stays the system of
record until a clean month and explicit cutover approval.

See [`plan/STATUS.md`](plan/STATUS.md) for where things stand and
[`plan/roadmap.md`](plan/roadmap.md) for where it's going.

## Quickstart

```bash
cargo test          # whole suite
cargo run -p ledger-mcp -- --data-dir /tmp/books   # MCP server over stdio
cargo run -p ledger-gui -- --demo                   # the viewer, on a demo company
```
New books, one command (installs Rust and wanderbooks if needed, makes the
books' private folder, starts books called `acme` and lets Claude Code reach
them; nothing needs sudo):

```bash
curl -fsSL https://wanderbooks.ai/install.sh | sh -s -- acme
```

Already installed, three commands: `wanderbooks setup`, `wanderbooks books
create acme`, `wanderbooks connect claude --book acme` (or `connect codex`).
Then talk to the agent; `wanderbooks gui acme` shows the books, and its
Books menu backs them up, restores, starts new books and closes them.

## Developing

Every tool the recipes use (Rust, just, Kani, Lean, cargo-mutants,
cargo-fuzz, Node for deploys) is pinned in `mise.toml`:

```sh
mise install
just          # lists the recipes: check, build, prove, demo, ...
```

## Layout

```
crates/ledger-core/    # engine: money, accounts, journal, ledger, reports
crates/ledger-store/   # journal + SQLite projection + blob store
crates/ledger-mcp/     # MCP server: 119 tools, resources, handbook, receipt policy
crates/ledger-import/  # QBO CSV import, IIF/XLSX export
crates/ledger-config/  # wanderbooks.toml company settings
crates/ledger-cli/     # `wanderbooks`: the one executable (every mode)
crates/ledger-host/    # the host, the agents' bridge, the owner's commands
crates/ledger-package/ # The CPA package and the 1099 worksheet (server and viewer)
crates/ledger-gui/     # the viewer: read-only, live (egui)
docs/                  # requirements, architecture, oracle, parity, glossary
plan/                  # STATUS.md, roadmap.md
AGENTS.md              # operating manual for agents working here
```

## License

MIT OR Apache-2.0. See `LICENSE-MIT` and `LICENSE-APACHE`.
