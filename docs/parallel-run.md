# Parallel-run readiness

The Google Sheet stays the system of record until the parallel run passes
and the owner explicitly approves cutover. This doc is the plan; the run
itself needs the owner's go-ahead.

## Prerequisites (all met)

- Capability parity 12 of 12 rows covered (`docs/capability-parity.md`).
- Engine semantics verified: 70 workspace tests, property tests, mutation
  testing, 11.5M fuzz executions with zero crashes.
- Storage: hash-chained JSONL journal (source of truth), SQLite as
  disposable projection with auto-rebuild, content-addressed blob store.
- Export: CSV/JSON/XLSX/IIF for CPA handoff and QuickBooks Desktop.

## The run

1. **Duration:** one full calendar month.
2. **Dual entry:** every transaction goes into BOTH the Sheet and
   wanderbooks (via the MCP tools). Receipts attach in both.
3. **Month-end diff:** export trial balance, P&L, balance sheet, and cash
   flow from wanderbooks (XLSX); compare against the Sheet's tabs.
   Every line must match to the cent.
4. **Discrepancy protocol:** any mismatch is investigated before the next
   month starts. If the engine is wrong, fix the engine (with a test).
   If the Sheet is wrong, note it — the Sheet is the record until cutover.
5. **Cutover:** only after a clean month AND the owner's explicit approval.
   Cutover means: the Sheet becomes read-only history; wanderbooks becomes
   the system of record. The journal's git backup becomes the archive.

## What the agent does during the run

- Daily: `books.digest` review of what was recorded.
- Weekly: trial-balance export, eyeball against the Sheet.
- Month-end: full four-statement export + line-by-line diff.
- Every discrepancy gets a written explanation before proceeding.

## Rollback

If the run fails: the Sheet never stopped being the record. Delete the
wanderbooks data dir, fix the issues, restart the month. No data loss —
the Sheet has everything.
