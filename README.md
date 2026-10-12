# wanderbooks

Canonical double-entry bookkeeping, rebuilt from scratch. The engine is
verified against oracles, parity is tracked automatically, and the primary
interface is AI-first: you talk to an agent, the agent drives the books
through MCP. Self-hosted, no rent.

## What it is

- **`ledger-core`** — the semantics engine. Exact decimal money, five
  account types, balanced journal entries, immutable posted log with
  reversing entries, trial balance, P&L, balance sheet, indirect-method
  cash flow, fixed-asset register with straight-line depreciation.
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
- **`ledger-gui`** — the read-only desktop view for owners and their
  accountant (egui). Answers first (cash, runway, profit, what needs
  attention), then every number drills down to its lines, its entry, the
  rendered receipt, and the journal history. Follows the journal live:
  a change the agent makes appears in milliseconds. `--demo` opens a
  realistic demo company.

## Verification

This is the storytold way applied to accounting: agents verify against
oracles, never ship what they haven't demonstrated.

- 70 workspace tests: invariant tests encoding accounting law, golden-file
  report snapshots, property tests (proptest), end-to-end MCP stdio tests.
- Mutation testing via cargo-mutants; fuzzing via cargo-fuzz (11.5M+
  executions, zero crashes).
- GnuCash is the behavioral oracle (never a code source — clean room,
  MIT OR Apache-2.0).

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
cargo run -p ledger-gui -- --demo                   # the desktop view, on a demo company
```
New books, one command (installs Rust and wanderbooks if needed, makes the
books dir with the MCP server registered for it, and starts the agent):

```bash
curl -fsSL https://git.swaits.com/swaits/wanderbooks/raw/branch/main/install.sh | sh -s -- ~/Books/Acme
```

Already installed: `wanderbooks new ~/Books/Acme`. In a books dir, `just`
lists the shortcuts (`just gui`, `just agent`, `just backup`).

## Layout

```
crates/ledger-core/    # engine: money, accounts, journal, ledger, reports
crates/ledger-store/   # journal + SQLite projection + blob store
crates/ledger-mcp/     # MCP server: 64 tools, resources, receipt policy
crates/ledger-import/  # QBO CSV import, IIF/XLSX export
crates/ledger-config/  # wanderbooks.toml company settings
crates/ledger-cli/     # `wanderbooks new`: one-command books dir + agent
crates/ledger-gui/     # read-only live desktop view (egui)
docs/                  # requirements, architecture, oracle, parity, glossary
plan/                  # STATUS.md, roadmap.md
AGENTS.md              # operating manual for agents working here
```

## License

MIT OR Apache-2.0. See `LICENSE-MIT` and `LICENSE-APACHE`.
