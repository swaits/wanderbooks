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
| A/R invoices + payments + aging | Business → Invoices | done | R-FUTURE-5; since P4b S1 (D-45) a document's paid, outstanding and status are read from its tagged ledger lines, hand lines on the receivables account are refused, `books.void_document` and `books.control_check` tie the subledger to the control account (rules.md B-38); S2 `books.documents`/`books.document` and the viewer pages; S3 the invoice PDF; S4.1-S4.3 terms, numbers, line accounts, the issued PDF filed at issue (behind `documents_v2`, the owner's next upgrade); numbers, terms, line accounts, the party snapshots and the issued PDF filed with the invoice, through `books.create_invoice`'s preview and confirm (P4b S4.4); S4.5 the issued PDF carries the number, terms, PO, quantity and price, the bill-to's contact block and ship-to, the ways to pay with QR codes, the memo and the footer, and the viewer shows the number, terms, bill-to and PO and saves the issued copy |
| A/P bills + payments + aging | Business → Bills | done | R-FUTURE-5; the same S1 derivation and control guard on the payables account; S4.1-S4.3 the vendor's invoice number, terms, every line on its own account (behind `documents_v2`); the vendor's own invoice number (a duplicate refused), terms, line accounts and the vendor's invoice filed as the bill's evidence, through `books.create_bill`'s preview and confirm (P4b S4.4); S4.5 the viewer shows the vendor's number, the day received, terms and memo, and the handbook's procedure (numbers, terms, line accounts) |
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

### P4b S4.5: the issued PDF, the viewer and the handbook (2026-10-29)

The A/R and A/P rows above gain their note rather than a new row: no
parity item changed status, so the floor stands. The invoice a customer
pays from now prints its number, both dates, the terms (an early
discount reads in them), the customer's purchase order, each line's quantity beside its
price, the customer as it was at issue (contact, billing address, phone,
email and a Ship-to block), the company's email and website, the ways to
pay (each with a QR code when it has a link), the memo and the footer. The
current copy says it was re-created as of a day (never the wall clock).
The viewer's invoice page shows the number, terms, bill-to and PO and
saves the issued copy beside the re-created one; the Money in list names an
invoice by its number. The handbook's invoices-and-bills procedure is
complete (issue, collect, pay on terms, void or write off, the fee, the
CPA's list) and says what waits for the owner's upgrade. rules.md gains
B-39 (terms) and B-40 (numbers and the issued copy). No golden moved.

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

### P4: drafts, the evidence gate, the preview digest (2026-10-29)

Three new rows, none above changed status, and the floor stands.

| Area | Counterpart (a cross-check, not the spec) | Status | Notes |
|---|---|---|---|
| Draft states and discard (`books.set_draft_state`, `books.discard_draft`, `books.list_drafts`; Lean `Step.discard`) | Draft/pending transactions (QBO "pending"), voiding an unposted entry | partial | `entry.draft_state_set` and `entry.draft_discarded` under `drafts_v2`; B-43; tests `discard_unblocks_close`, `draft_states_replay_and_project`, `a_draft_waits_for_its_receipt_and_posts`; the fixture `p4_before` was written by the version before and opens to the same figures; `partial` until a hand-verified golden covers a discard |
| One evidence gate (`evidence::requirement`) | Attach-required rules on posting | partial | the store, the previews, the digest, the viewer and Needs attention call one function; `gui_and_server_gate_agree`, `a_reversal_needs_no_receipt_of_its_own` (D-47); same tests as above for the golden |
| Preview digest on every confirm (D-50) | Confirm dialogs | partial | `stale_preview_refused`, `every_confirm_tool_takes_a_preview`; detects drift, not a human's look |

The digest also gained `since_seq` (what was posted since a journal line,
backdated entries included: defect 6), `zero_net_accounts`,
`drafts_by_state` and the reconciliation split; reports gained
`as_of_status` and `strict`; the CPA package is refused while a draft
dated in it is open (4.8).

### P5: the agent surface (2026-10-29)

No semantic feature moved, so no status changed and the floor stands. P5
changes how an agent is told what to do, not how a figure is computed: the
result envelope on every tool result (`next_steps`, `guards`, `warnings`,
`head`), `books.start` v2, the prompts, the resource templates,
`books.journal_history` (the journal in words) and source corrections
(`source_revisions`: `convention.source_revised`, `decision.source_revised`,
`open_item.source_revised`). One new row, `partial` until a hand-verified
golden covers it:

| Area | Counterpart (a cross-check, not the spec) | Status | Notes |
|---|---|---|---|
| Audit trail in words (`books.journal_history`, the `describe` sentences) | QBO "Audit log", Xero "History and notes" | partial | tests `journal_history_traces_what_the_books_did`, `every_event_type_has_a_sentence`; the viewer keeps its own sentences (the shared describer is the server's) |
### P8: audit and attestations (2026-10-29)

Three new rows, none above changed status, and the floor stands.

| Area | Counterpart (a cross-check, not the spec) | Status | Notes |
|---|---|---|---|
| Audit run: one programme of checks over the statements, the control accounts, closes, schedules, assets, evidence, drafts, tax categories, 1099 decisions, settings and the store's integrity (`books.audit`, `books.workpaper`) | An auditor's workpaper; QBO "Audit log"/"Review" and Xero's "Reports review" are a cross-check only | partial | rules.md B-45; `ledger_core::audit`, `ledger_store::audit`; tests `audit_detects_each_seeded_defect` (every code seeded), `audit_is_deterministic`, `workpaper_pdf_is_deterministic`; the fixture `p8_before` was written by the version before; `partial` until a hand-verified golden covers a workpaper |
| Attestations of receipts, entries and periods, weighed against `policy.attestation_bar` when asked (`books.attest_receipt`, `books.review_entry`, `books.attest_period`, `books.attestations`) | An accountant's sign-off on a month | partial | `below_bar_not_counted`, `amount_mismatch_never_counts`, `stale_head_period_attestation_refused`; the method is the client's own claim, and the books say so |
| The CPA package waits for a counted audit (`policy.cpa_package_requires_audit`) | A close checklist before handoff | partial | `cpa_package_refuses_without_a_counted_audit_unless_a_reason_is_given`; behind the feature `audits` |

## P11b: the agent dock, books homes, first run (2026-10-29)

| Area | Counterpart (a cross-check, not the spec) | Status | Notes |
|---|---|---|---|
| Books home: a visible folder per book, `<Documents>/Wanderbooks/<id>`, holding `AGENTS.md`, `CLAUDE.md`, `.mcp.json` and `.claude/settings.json` and nothing of the books | A QBO/Xero company has no counterpart; the point is the agent's workplace (D-27) | partial | `books_home_holds_no_books_data`, `a_books_home_made_again_keeps_what_a_person_kept` (`crates/ledger-host/tests/books_home.rs`); `partial` until the macOS home is tried |
| Agent dock: Claude Code or Codex in a terminal along the bottom of the viewer, started in the books home; never required | QBO's assistant panel is a cross-check only | partial | `dock_runs_the_chosen_agent`, `agent_missing_offers_the_installer` (`crates/ledger-gui/tests/dock.rs`); Linux checked; macOS by the CI run; Windows not offered (ConPTY unchecked) |
| First run with no terminal: the viewer sets up the private folder, the owner passphrase, the first books and an agent | QBO's "set up your company" flow | partial | `first_run_onboards_with_no_terminal` (`crates/ledger-gui/tests/welcome.rs`); the AppImage's whole path is the EM's to try on a clean machine |
