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
| Reversing entries | Reverse transaction | done | linked via `reversal_of` |
| Canonical report snapshots (JSON) | — (our oracle format) | done | `reports::snapshot` |
| Fixed-asset register + straight-line depreciation | Fixed Asset register | done | `assets.rs`; golden-covered schedule; idempotent runs; disposal with gain/loss |
| Cash flow statement (indirect) | Reports → Cash Flow | todo | R-FUTURE-3 |
| Period close / retained earnings | Close books | todo | R-FUTURE-2 |
| Multi-currency + FX translation | Multi-currency accts | todo | R-FUTURE-1 |
| QuickBooks CSV import (bank txns, chart of accounts) | Banking → Upload | todo | R-FUTURE-4 |
| QuickBooks IIF + XLSX export | Export / reports | todo | R-FUTURE-4 |
| A/R invoices + payments + aging | Business → Invoices | todo | R-FUTURE-5 |
| A/P bills + payments + aging | Business → Bills | todo | R-FUTURE-5 |
| Budgets | Budgets | todo | R-FUTURE-6 |
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
