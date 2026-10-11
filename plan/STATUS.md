# STATUS — 2026-10-10

## Where we are

Phase 1 (semantics core) is underway. The repo exists, is private, and the
first engine slice is built, tested, and green.

**Done:**
- Repo `swaits/wanderbooks` created on git.swaits.com (private).
- `ledger-core` 0.1.0: `Money` (exact Decimal + ISO currency), five account
  types with normal sides, chart of accounts (codes, hierarchy,
  placeholders), journal entries with full validation, `Ledger` command
  surface (create/add-line/post/reverse), append-only posting log.
- Reports: trial balance, P&L, balance sheet (accounting equation holds;
  current earnings folded into equity pre-close), canonical JSON `snapshot()`
  for oracle diffing.
- 14 tests green: validation rejections, posted-entry immutability,
  reversal semantics, the accounting laws on a fixture company, a 300-entry
  randomized invariant test, and the first golden file.
- Docs: AGENTS.md (operating manual), requirements, architecture, oracle
  design, parity catalog (floor: 6 done), glossary. Plan: this file + roadmap.

**In progress:** nothing — the next arcs are queued below, unstarted.

## Next arcs (in order)

The project went AI-first on 2026-10-10 (Stephen's call): the primary
interface is MCP, not a cloned GUI. The engine roadmap (period close, cash
flow, proptest) continues as needed, but the next *phase* is the interface:

1. **`ledger-store`** (`docs/receipt-store.md`): SQLite persistence,
   append-only triggers, TEXT amounts, event log, migrations,
   rebuild-from-log verify.
2. **Blob store**: content-addressed receipts/documents, attachment metadata,
   Drive backfill script.
3. **`ledger-mcp`** (`docs/mcp-interface.md`): stdio transport first;
   narrow tools with preview-before-post; digest; read-only resources; the
   agent operating contract in the tool descriptions.
4. **Capability parity** (`docs/capability-parity.md`): build every specced
   row, then the month-long parallel run against the GSheet; cutover per
   the cutover rule.
5. Then: oracle harness (GnuCash diff), read-only GUI on the event log,
   workflow parity.

## Decisions log

- 2026-10-10: Single currency per ledger for day one (R-5). Multi-currency
  is designed-for, not built.
- 2026-10-10: `Money` arithmetic is checked and returns `Result`; no
  panicking ops, no float paths anywhere.
- 2026-10-10: Negative zero is normalized away at `account_balance` — money
  never prints "-0". (Caught by the golden test on day one; the oracle works.)
- 2026-10-10: License MIT OR Apache-2.0, matching the storytold projects.
  GnuCash clean-room boundary documented in `docs/oracle.md`.
- 2026-10-10: AI-first pivot (Stephen). Primary interface is MCP
  (`docs/mcp-interface.md`), not a cloned GUI. Receipts/documents become a
  content-addressed blob store (`docs/receipt-store.md`). Parity target for
  the phase is the GSheet+GDrive system (`docs/capability-parity.md`), with
  a parallel-run cutover rule. GUI drops to read-only, event-log-driven,
  phase 4.
- 2026-10-10: Import/export targets QuickBooks formats (CSV, IIF, XLSX) —
  "could not give a single shit about GnuCash XML." GnuCash stays the
  *behavioral* oracle only. Receipt enforcement is server-side policy in the
  MCP layer (threshold + per-account overrides, typed `receipt_required`
  error, checklist in every preview); the engine stays policy-free.
- 2026-10-10: Storage durability (Stephen). SQLite is the query engine, not
  the source of truth: the source of truth is an append-only, hash-chained
  JSONL journal (fsync per append; tamper-evident; git-backup-friendly),
  SQLite is a disposable projection rebuilt from it, blobs stay
  content-addressed. Single write path: journal → fsync → apply to SQLite.
