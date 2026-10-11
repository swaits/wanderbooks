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

1. **Period close** (R-FUTURE-2): fiscal periods, closing entries moving P&L
   to retained earnings, locked periods. Parity: balance sheet then reports
   retained earnings instead of folded current earnings.
2. **Cash flow, indirect method** (R-FUTURE-3): from P&L + balance-sheet
   deltas. Golden-covered.
3. **Property tests graduate**: `proptest` with shrinking for the randomized
   invariant test; new properties for close and cash flow.
4. **GnuCash XML import** (R-FUTURE-4, read-only): parse GnuCash files into
   fixture books. This unblocks the oracle harness.
5. **Oracle harness** (`ledger-oracle`, `cargo xtask oracle`): run fixture
   books through our engine and GnuCash headless, diff snapshots
   automatically. Parity catalog becomes machine-checked; CI enforces the
   floor.
6. **egui app shell** (`books-app`): command-driven UI, native + WASM,
   screenshot-in-the-loop per the storytold protocol. Workflow parity catalog
   (QuickBooks/Xero reference) starts here.

## Decisions log

- 2026-10-10: Single currency per ledger for day one (R-5). Multi-currency
  is designed-for, not built.
- 2026-10-10: `Money` arithmetic is checked and returns `Result`; no
  panicking ops, no float paths anywhere.
- 2026-10-10: Negative zero is normalized away at `account_balance` — money
  never prints "-0". (Caught by the golden test on day one; the oracle works.)
- 2026-10-10: License MIT OR Apache-2.0, matching the storytold projects.
  GnuCash clean-room boundary documented in `docs/oracle.md`.
