# Parity catalog

Every semantic feature we intend to clone, with its status. Updated in the
same commit as the feature. **The floor never decreases**: the count of
`done` items is monotonic across commits.

Statuses: `done` = implemented + invariant-tested + golden-covered.
`partial` = implemented but missing tests or goldens. `todo` = not started.
No other statuses.

**Floor: 10 done** (2026-10-10; fixed-asset register added).

| Area | GnuCash counterpart (a cross-check, not the spec) | Status | Notes |
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
| Fixed-asset register + straight-line depreciation | Fixed Asset register | done | `assets.rs`; golden-covered schedule; idempotent runs; disposal with gain/loss; posted (the books') vs scheduled, in-service vs acquired dates, the §723, depreciable and §704(c) figures (P3) |
| Cash flow statement (direct, with the reconciliation schedule and noncash disclosures) | Reports → Cash Flow | done | R-FUTURE-3; rules.md B-14 (P3 defect 1) |
| Foreign-currency lines (base + foreign stored) | Multi-currency accts | done | R-FUTURE-1 |
| Period close (closing entry to Retained Earnings) | Close books | done | R-FUTURE-2 |
| Bank reconciliation (cleared flags + report) | Reconcile | done | — |
| QuickBooks CSV import (bank txns, chart of accounts) | Banking → Upload | done | R-FUTURE-4 |
| QuickBooks IIF + XLSX export | Export / reports | done | R-FUTURE-4 |
| A/R invoices + payments + aging | Business → Invoices | done | R-FUTURE-5 |
| A/P bills + payments + aging | Business → Bills | done | R-FUTURE-5 |
| Payroll runs (accounting only, no tax tables) + tax remittance | Payroll | done | — |
| Tracking classes (divisional P&L) | Class tracking | done | invoice and bill lines take a class too (projects, rules.md B-37 item 8; QBO/Xero class or tracking per line) |
| CPA handoff package (TB + GL xlsx, P&L + BS pdf, zip) | Reports → export | done | — |
| Budgets | Budgets | todo | R-FUTURE-6 (declined as bloat) |
| Scheduled transactions (recurring entries → drafts) | Scheduled txns / Recurring transactions | partial | hand-verified fixture incl. month-end dates; drafts only, by design |
| Payout gross-up (deposit → revenue, fees, adjustments) | Bank deposit with fee lines; Xero/QBO Stripe & Shopify integrations | partial | hand-verified fixture incl. classes and a negative payout; no statement parsers yet |
| 1099 vendor tracking + year-end worksheet | Vendors → Track payments for 1099; 1099 report | partial | hand-verified fixture (bill payments, card exclusion, reversal, year boundary); no golden file yet |
| Prepaid expense + deferred revenue amortization | Amortization schedules (Xero/QBO add-ons) | partial | hand-verified fixture, to the cent; no golden file yet |
| Automated cross-implementation diff harness (GnuCash first) | — (our `xtask oracle`) | todo | roadmap phase 3 |

Workflow/UI parity (QuickBooks/Xero reference) gets its own catalog when the
egui phase starts; semantics come first.

### P0 guard rails (2026-10-16)

P0 (`plan/served.md`) changed no feature in the catalog above, so no
status moved and the floor stands. It hardened what exists: an older
version refuses books from a newer one as newer, not corrupt (rules.md
B-27); the MCP server fails closed after an internal error, and no input
reaches one (dates are four-digit years, amounts at most a line's, B-28);
local files
were read only from the import roots, never the books, the vault or its
key (P2 slice 5 removed local paths: files now come in only as uploads,
D-46); no file with a full TIN is filed into the books (B-26); the agent is
told the rules in force in a short connect text and a handbook, and no
agent text points at the books' files, git or the repo.

## U0b: one attention engine (D-53, 2026-10-26)

No semantic feature changed, so no status moved and the floor stands (10
done). Needs attention used to be computed by the viewer alone; it now
lives in `ledger-package::attention`, over `PackageBooks`, and both the
viewer and `books.start` read it (the same items, the same prompts), so the
list the human sees and the next steps the agent gets cannot drift. The
parity test `the_viewer_and_books_start_see_the_same_attention` compares
them item for item on the demo books and on a book written entry by entry.

## Capability parity (AI-first phase)

Semantic parity (above) is about *correctness*: the engine computes the
right numbers. Capability parity is about *usefulness*: can the new system
do everything the GSheet+GDrive system does today? Tracked separately in
`docs/capability-parity.md`, with its own cutover rule (parallel run +
the owner's approval). The two catalogs converge: when both are green, the
sheet retires.

### P13 bookspec (2026-10-26)

P13 is a specification, not a feature, so no row of the semantic catalog
moves and the floor stands. It adds the checker crate (`bookspec/checker`:
the example schema, the rule identifiers `BS-<area>-<n>` and the `Engine`
trait), the runner (`bookspec/runner`: pass, fail or not claimed per rule),
and the rule index from B-1 to B-45 (`bookspec/spec/rules-index.md`). Every
existing bookspec example still runs on every `cargo test`, through both
the loader's own checks and the `Engine` trait.

## P6: the evidence pipeline (2026-10-26–27)

No semantic feature of the catalog above moved, so no status changed and
the floor stands (10 done): P6 changes how evidence is kept, not how a
figure is computed. It adds one in-process extractor (a PDF's text and
pages, an XLSX's cells as exact decimal strings, plain text; 25 MiB, 30
pages, 2 MiB of text), journals each extraction as its own record (one per
attachment, extractor and version, its text a content-addressed blob whose
hash `books.verify` checks), holds identical bytes as one live attachment
(linked to every subject, legacy duplicates consolidated with
`duplicate_of`), records what a human read off a file (the closed
annotation field list, engine receipt numbers running from the max) and
what a payment settles (`entry.settles`, the only source of "reimbursed"),
and gives company records four more homes (`notices`, `correspondence`,
`plans`, `history`, D-20). Rules.md B-44 states it; `docs/testing.md`'s P6
section lists its tests and the P12 mutation targets.
