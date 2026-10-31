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
- **`ledger-mcp`** — the AI-first interface. 122 MCP tools (2026-10-28), served by the
  books server (`wanderbooks server`) on the data root's local sockets and
  reached through the agents' bridge (`wanderbooks mcp --book ID`): draft →
  preview → post workflow, server-side receipt
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

- About 2,950 tests across the two workspaces (2026-10-29): invariant tests encoding accounting law, golden-file
  report snapshots, property tests (proptest), end-to-end MCP tests through
  a real books server.
- **Formal proofs of the accounting laws** (`just prove`,
  `docs/verification.md`). Kani model-checks the engine's arithmetic
  kernel exhaustively within bounds. Lean 4 proves that, after any
  sequence of commands, the trial balance balances, the accounting
  equation holds, posted entries are forever, closed periods stay
  frozen, and a close moves exactly the net income to equity. The first
  pass found three engine bugs that every test, fuzzer, and mutant had
  missed: negative amounts accepted, and two crashes on oversized
  amounts. It also caught a soundness bug in the model checker itself.
- Mutation testing via cargo-mutants, a local run (paused for development
  until the P12 max-testing pass, D-33; not a CI job, D-59); fuzzing via
  cargo-fuzz (11.5M+ executions, zero crashes), weekly in CI.
- GnuCash is *a* behavioral cross-check, never the authority and never
  a code source (clean room, MIT OR Apache-2.0). Accounting principles,
  proved, are the spec. Where any other program disagrees, we work out
  from principles who is right.

## Status

Parity is 12 of 12 against the spreadsheet workflow it replaced
(`docs/capability-parity.md`); the books cut over on 2026-10-20 and the
Sheet is frozen as of 2026-10-11. The remaining work, its order and its
hand-off are `plan/served.md` section 8.0; the destination is one
installed app for Mac, Windows and Linux (P11), then the hosted service
the plan describes and does not build. Today the agent path runs on
Linux, macOS and Windows from a source build: tokens live in a file under
the private data root (D-63), the data root is `%LOCALAPPDATA%\wanderbooks`
with an owner-only access list on Windows (P11c, D-62), and the books'
local channels are named pipes there. Windows is not yet packaged: no
installer, no signing (P11a).

See [`plan/STATUS.md`](plan/STATUS.md) for where things stand and
[`plan/roadmap.md`](plan/roadmap.md) for where it's going.

## Quickstart

```bash
cargo test                         # whole suite
cargo run -p ledger-gui -- --demo  # the viewer, on a demo company
```
New books, one command (installs Rust and wanderbooks if needed, makes the
books' private folder, starts books called `acme` and lets Claude Code reach
them; nothing needs sudo):

```bash
curl -fsSL https://wanderbooks.ai/install.sh | sh -s -- acme
```

(Linux and macOS. On Windows build from source: `cargo install --path crates/ledger-cli`, then the three commands below.)

Already installed, three commands: `wanderbooks setup`, `wanderbooks books
create acme`, `wanderbooks connect claude --book acme` (or `connect codex`).
Then talk to the agent; `wanderbooks gui acme` shows the books, and its
Books menu backs them up, restores, starts new books and closes them. With
no terminal (the AppImage, a menu entry), opening the app with no books
shows a first-run page that does the same three steps (the private folder,
an owner passphrase, the first books) and connects an agent; a panel along
the bottom of the window then runs Claude Code or Codex in the books' own
folder, `~/Documents/Wanderbooks/<id>` (optional: any agent can use the
books).

Updating: an AppImage, or a `wanderbooks` you put somewhere yourself, brings
itself to the newest release with `wanderbooks update` (it checks the
download against our release key, keeps the version it replaces, and the
open books restart onto the new one when idle); `wanderbooks rollback` goes
back, unless a book has since moved to a newer journal format. A `.deb`,
`.rpm` or AUR install is updated by its package manager and `update` says
so. The Linux packages are built here with `just package-linux`
(docs/packaging.md); none is published yet.

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
crates/ledger-mcp/     # MCP server: 129 tools, resources, handbook, receipt policy
crates/ledger-import/  # QBO CSV import, IIF/XLSX export
crates/ledger-config/  # wanderbooks.toml company settings
crates/ledger-cli/     # `wanderbooks`: the one executable (every mode)
crates/ledger-host/    # the host, the agents' bridge, the owner's commands
crates/ledger-package/ # the shared read-only view: the CPA package, the invoice PDF, Needs attention
crates/ledger-vault/   # the TIN vault (XChaCha20-Poly1305 under the book's key)
crates/ledger-gui/     # the viewer: read-only, live (egui)
verify/lean/           # the accounting laws as Lean theorems (`just prove`)
# the formal accounting specification lives in its own repository, https://git.swaits.com/swaits/bookspec (pinned dev-dependency, P13)
fuzz/, oracle/         # fuzz targets; the GnuCash differential (`just oracle`)
deploy/                # the self-host recipe and system unit (P9a)
site/                  # wanderbooks.ai and install.sh
docs/                  # requirements, architecture, oracle, parity, testing, verification, spec, glossary
plan/                  # served.md (the plan), STATUS.md, roadmap.md, HANDOFF.md, gui.md
AGENTS.md              # operating manual for agents working here
```

## License

MIT OR Apache-2.0. See `LICENSE-MIT` and `LICENSE-APACHE`.
