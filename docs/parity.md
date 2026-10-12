# Parity catalog

Every semantic feature we intend to clone, with its status. Updated in the
same commit as the feature. **The floor never decreases**: the count of
`done` items is monotonic across commits.

Statuses: `done` = implemented + invariant-tested + golden-covered.
`partial` = implemented but missing tests or goldens. `todo` = not started.
No other statuses.

**Floor: 10 done** (2026-10-10; fixed-asset register added).

| Area | GnuCash reference | Status | Notes |
|---|---|---|---|
| Chart of accounts (5 types, codes, hierarchy) | Accounts tab | done | `account.rs`; placeholder support in |
| Double-entry validation (balanced, ≥2 lines) | Journal validation | done | `journal.rs::validate` |
| Exact decimal money, no floats | — (we go further) | done | `money.rs`; Currency ISO codes |
| Posted-entry immutability | Read-only posted txns | done | reversal via `reverse_entry` |
| Trial balance | Reports → Trial Balance | done | golden-covered |
| Balance sheet (A = L + E) | Reports → Balance Sheet | done | current earnings folded in pre-close; golden-covered |
| Profit & loss | Reports → Income Statement | done | golden-covered |
| P&L for a date range, line per account (closing entries excluded) | Reports → Income Statement (custom dates) | partial | hand-verified fixture + property tests; no golden file yet |
| Balance sheet as of a date, line per account | Reports → Balance Sheet (as of) | partial | hand-verified fixture + property tests; no golden file yet |
| General ledger by account (running balance, split) | Reports → General Ledger | partial | hand-verified fixture, ties to trial balance; no golden file yet |
| Trial balance as of a date | Reports → Trial Balance (as of) | partial | hand-verified fixture; no golden file yet |
| Statement of changes in members' equity | Reports → Statement of Changes in Equity | partial | hand-verified two-member fixture incl. rounding and a close; no golden file yet |
| Tax line mapping + tax summary | Accounts → Tax line mapping; Tax reports | partial | hand-verified fixture (50% meals, nondeductible, contra income); no golden file yet |
| Reversing entries | Reverse transaction | done | linked via `reversal_of` |
| Canonical report snapshots (JSON) | — (our oracle format) | done | `reports::snapshot` |
| Fixed-asset register + straight-line depreciation | Fixed Asset register | done | `assets.rs`; golden-covered schedule; idempotent runs; disposal with gain/loss |
| Cash flow statement (indirect) | Reports → Cash Flow | done | R-FUTURE-3 |
| Period close / retained earnings | Close books | todo | R-FUTURE-2 |
| Foreign-currency lines (base + foreign stored) | Multi-currency accts | done | R-FUTURE-1 |
| Period close (closing entry to Retained Earnings) | Close books | done | R-FUTURE-2 |
| Bank reconciliation (cleared flags + report) | Reconcile | done | — |
| QuickBooks CSV import (bank txns, chart of accounts) | Banking → Upload | done | R-FUTURE-4 |
| QuickBooks IIF + XLSX export | Export / reports | done | R-FUTURE-4 |
| A/R invoices + payments + aging | Business → Invoices | done | R-FUTURE-5 |
| A/P bills + payments + aging | Business → Bills | done | R-FUTURE-5 |
| Payroll runs (accounting only, no tax tables) + tax remittance | Payroll | done | — |
| Tracking classes (divisional P&L) | Class tracking | done | — |
| CPA handoff package (TB + GL xlsx, P&L + BS pdf, zip) | Reports → export | done | — |
| Budgets | Budgets | todo | R-FUTURE-6 (declined as bloat) |
| Scheduled transactions | Scheduled txns | todo | R-FUTURE-6 |
| Automated GnuCash diff harness | — (our `xtask oracle`) | todo | roadmap phase 3 |

Workflow/UI parity (QuickBooks/Xero reference) gets its own catalog when the
egui phase starts; semantics come first.

## Capability parity (AI-first phase)

Semantic parity (above) is about *correctness*: the engine computes the
right numbers. Capability parity is about *usefulness*: can the new system
do everything the GSheet+GDrive system does today? Tracked separately in
`docs/capability-parity.md`, with its own cutover rule (parallel run +
the owner's approval). The two catalogs converge: when both are green, the
sheet retires.
