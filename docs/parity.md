# Parity catalog

Every semantic feature we intend to clone, with its status. Updated in the
same commit as the feature. **The floor never decreases**: the count of
`done` items is monotonic across commits.

Statuses: `done` = implemented + invariant-tested + golden-covered.
`partial` = implemented but missing tests or goldens. `todo` = not started.
No other statuses.

**Floor: 21 done** (2026-10-28; it was 10 on 2026-10-10, when the fixed-asset register was added, and the sections below record each rise).

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
| A/R invoices + payments + aging | Business → Invoices | done | R-FUTURE-5; since P4b S1 (D-45) a document's paid, outstanding and status are read from its tagged ledger lines, hand lines on the receivables account are refused, `books.void_document` and `books.control_check` tie the subledger to the control account (rules.md B-38); S2 `books.documents`/`books.document` and the viewer pages; S3 the invoice PDF; S4.1-S4.3 terms, numbers, line accounts, the issued PDF filed at issue (behind `documents_v2`, the owner's next upgrade); numbers, terms, line accounts, the party snapshots and the issued PDF filed with the invoice, through `books.create_invoice`'s preview and confirm (P4b S4.4) |
| A/P bills + payments + aging | Business → Bills | done | R-FUTURE-5; the same S1 derivation and control guard on the payables account; S4.1-S4.3 the vendor's invoice number, terms, every line on its own account (behind `documents_v2`); the vendor's own invoice number (a duplicate refused), terms, line accounts and the vendor's invoice filed as the bill's evidence, through `books.create_bill`'s preview and confirm (P4b S4.4) |
| Payroll runs (accounting only, no tax tables) + tax remittance | Payroll | done | — |
| Tracking classes (divisional P&L) | Class tracking | done | invoice and bill lines take a class too (projects, rules.md B-37 item 8; QBO/Xero class or tracking per line) |
| CPA handoff package (TB + GL xlsx, P&L + BS pdf, zip) | Reports → export | done | — |
| Budgets | Budgets | todo | R-FUTURE-6 (declined as bloat) |
| Scheduled transactions (recurring entries → drafts) | Scheduled txns / Recurring transactions | partial | hand-verified fixture incl. month-end dates; drafts only, by design |
| Payout gross-up (deposit → revenue, fees, adjustments) | Bank deposit with fee lines; Xero/QBO Stripe & Shopify integrations | partial | hand-verified fixture incl. classes and a negative payout; no statement parsers yet |
| 1099 vendor tracking + year-end worksheet | Vendors → Track payments for 1099; 1099 report | partial | hand-verified fixture (bill payments, card exclusion, reversal, year boundary); no golden file yet |
| Prepaid expense + deferred revenue amortization | Amortization schedules (Xero/QBO add-ons) | partial | hand-verified fixture, to the cent; no golden file yet |
| Automated cross-implementation diff harness (GnuCash first) | — (`just oracle`) | partial | live: `tests/oracle_gnucash.rs` runs 200 random books through GnuCash 5.10 in a pinned container, 15,800 exact comparisons, weekly in CI (docs/oracle.md Layer 5); open: fixture books diffed as `snapshot()` output, and the machine-readable catalog with CI enforcing this floor |
| Invoice sent tracking (who, how, to whom, when, original or reminder, the channel's reference) | Business → Invoices (Xero's "Sent"; QBO's email) | done | P4b S5; records, never sends (A-4); a send changes no figure; `invoice.sent` under `documents_v2` (D-52) |
| Multi-document settlements with early-payment discounts (gross method) | Business → Invoices/Bills; Receive payment | done | P4b S6: one entry, one party, one application per document (`kernel::settle`, B-42); a discount beyond the terms needs a reason; a bill with an asset line is refused (D-49); the bill and its payment tested, replay equal to live |
| A processor's fee on a payment received (gross presentation) | Xero/QBO Stripe & Shopify deposits | done | P4b S6: Dr the deposit the net, Dr processing fees the fee, Cr A/R the gross (A-4); `a_fee_posts_gross_to_receivables_and_net_to_the_bank` |
| Bills due, aging detail and tie-out, aging exports | Business → Bills; Reports → A/R and A/P Aging | done | P4b S6: `books.bills_due`, `detail`, `control`, `ar_aging.csv`/`ap_aging.csv` in `books.export` and the CPA package, each with its `TOTAL` tie-out (`the_aging_csvs_tie_to_the_control_check`) |

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

## P4b: receivables and payables, first class (D-45; 2026-10-23 to 2026-10-27)

The two A/R and A/P rows above were `done` before P4b and stay `done`;
what moved is their notes, so the floor stands at 21. S1 changed what the
figures *are* (derived from tagged standing lines, so a reversed payment
no longer counts and a void is exact; proven by Lean `control_agrees`
and Kani `bounds_kept_is_the_definition_on_books_in_bounds`), S2 and S3
added the read tools, the viewer pages and the PDF, and S4.1 to S4.3
added terms, numbers, line accounts, the parties' snapshots and the
issued copy, journaled only when set so an old-shape document is byte
for byte the same. The rest of P4b (S4.4 and S4.5, S5 sent tracking, S6
paying on terms, S7 bad debts) is built on paused branches or open as
`plan/served.md` 8.0.1 says; each adds a note to these rows when it
lands. No golden moved.

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

### P4b S4.4: the create tools preview and confirm (2026-10-27)

The A/R and A/P rows above gain their note rather than a new row: no
parity item changed status, so the floor stands. `books.create_invoice`
and `books.create_bill` now preview without `confirmed` (the posting, the
number and due date it will take, the DRAFT or the discount, and nothing
written) and post with it (A-1), which is what the workflow reference
does before a document leaves the company. A bill in books that have
declared `documents_v2` names each line's account and keeps the vendor's
invoice (A-10, A-13); an invoice keeps the copy it issued (A-7, B-40) and
a `mailto` link for a human to send it, since the books never send (A-4).
`books.update_party` takes the party's email and terms per side (A-13) and
the owner's full contact block (D-55: phone, contact person, billing and
shipping addresses, billing email), which an invoice's bill-to copies at
issue.

### P9a self-host deployment (2026-10-26)

P9a is a deployment, not a semantic feature: the same engine, store, MCP
surface and reports on a machine of the owner's own. No row above changed
status and the floor stands. What it adds is the enforced isolation the
catalog's parity claims rest on in practice: a dedicated account, a data
root only that account can read, `books.start` saying
`service_user (enforced)` (`--isolation`), and the same bearer-guarded MCP
door on loopback for Caddy or a tunnel. `crates/ledger-cli/tests/two_user.rs`
proves the account boundary and the move's head in CI
(`.github/workflows/two-user.yml`); the recipe is `deploy/README.md`.
The cutover rule in `docs/capability-parity.md` is unchanged.

### P4b S5: sent tracking (2026-10-27)

The catalog gains one row: an invoice's recorded sends. The books never
send (A-4): `books.mark_sent` records who sent an invoice, how, to whom,
when, whether it was the original or a reminder, and the channel's own
reference, for an owner or a delegate such as an agent. The record is
`invoice.sent`, under `documents_v2` (D-52), and it moves no figure, so
the A/R and A/P rows' figures are unchanged. The floor never decreases:
the row is `done`, tested by `sends_replay_and_project`,
`a_send_changes_no_figure` and `mark_sent_records_who_and_how`. Sent
tracking also feeds Needs attention: an issued invoice nobody marked sent
(past `invoicing.unsent_days`) and an overdue invoice not reminded within
`invoicing.reminder_days` are one list in the viewer and in `books.start`,
each offering `books.mark_sent` (step 4; U0b's one engine).

### P4b S6 paying on terms (2026-10-27)

S6 adds the three rows above: multi-document settlements with
early-payment discounts (the gross method, A-8), the processor's fee
posted gross (A-4), and the bills-due list with the agings' detail,
tie-out and exports. The pre-S6 fixture's figures still hold (no figure
moves), and every new write needs the journal feature `documents_v2`,
which the owner's books declare at their next upgrade (D-52).
## P4b S7: bad debts (write-offs), 2026-10-27

A/R gained the direct write-off the owner chose (A-5; plan 8.2 S7): an
invoice that will not be collected comes off 1200 with Dr 6990 Bad Debt
Expense, its receivable line tagged `WriteOff` so the receipt's figures
stay derived, and its evidence of worthlessness filed in the same call
(rules.md B-41; IRC §166, §111; Treas. Reg. §1.166-1(e), (f), -2(a),
-3(a)). A same-year recovery is the reversal D-49 records, not a new
movement; bills are never written off. No row above changed status (the
A/R and A/P rows already say `done`), so the floor stands (10 done).
