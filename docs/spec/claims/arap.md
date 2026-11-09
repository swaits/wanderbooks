# Claims: arap

Wanderbooks' claims for bookspec area **arap**: what each rule requires, where the engine enforces it, and which tests check it. The accounting text lives in bookspec's records (`rules/arap/`); here stay the option wanderbooks picks, its enforcement and tests, and the product rules that are only wanderbooks' (`WB-arap-<n>`).

Rules: B-18, B-38, B-39, B-40, B-41, B-42.

---

## B-18 Aging buckets: Current, 1–30, 31–60, 61–90, 90+ days past due

**bookspec:** `BS-arap-1`

**Claims:** yes

**Pick:** a document due today is Current, not 1 day past due: CHOICE (the owner, 2026-10-13); an aging as of a day shows only the documents issued by then, less only the payments made by then: CHOICE (the owner, 2026-10-13); a payment never predates its invoice or bill, a real prepayment being a deposit: CHOICE (the owner, 2026-10-13). The bucket edges are the record's.
**Pick (BS-arap-7):** `due-today-is-current`, the owner, 2026-10-13.

**Pick (BS-arap-8):** `as-of-the-day`, the owner, 2026-10-13.

**Pick (BS-arap-9):** `refuse-a-payment-before-its-document`, the owner, 2026-10-13.

**Enforced:** `arap::aging_bucket`, `Ledger::ar_aging`/`ap_aging`
(through `Ledger::doc_state`), `receive_payment`/`pay_bill`
(`PaymentBeforeDocument`). **Checked:**
`aging_boundaries`, `aging_is_exclusive_and_only_ages`,
`aging_as_of_a_date_sees_only_what_existed_then`,
`a_payment_never_predates_its_document`, `aging_as_of_ignores_a_later_reversal`,
`a_voided_invoice_never_ages`, `status_is_a_function_of_journal_and_date`.

## B-38 A document's figures are its ledger lines that stand; a control account is its documents

**bookspec:** `BS-arap-2`

**Claims:** yes

**Pick (BS-arap-2):** `lines-that-stand`, the owner, 2026-10-23.

**Pick:** `lines-that-stand`: CHOICE (the owner, 2026-10-23, plan D-45: "we kinda need stronger invoice support. Like make it a first class citizen"; hand entries to receivables and payables refused, the owner's answer 5). A control account agreeing with its subledger is the CONVENTION beside it (as B-23, `BS-ledger-8`). The sources are cited and not yet vendored (the Open list in `README.md`).

**Enforced:** `ledger_core::arap::{DocRef, DocTag, Movement, DocState,
DocStatus, doc_status, PaymentDetails, ControlCheck}`,
`kernel::{running_bounds, bounds_kept}`, `Ledger::{line_doc, doc_lines,
doc_state, invoice_state, bill_state, stands_on, in_effect,
control_role_of, control_check, payments_recorded, standing_payments,
receive_payment_with, pay_bill_with, void_document, check_void_document,
check_reversal, replay_reverse_entry, ar_aging, ap_aging}` (the guard in
`add_line`, `add_line_fx`, `post_entry` and `register_recurring`);
`Store::{receive_payment_with, pay_bill_with, void_document}` (feature
`document_standing`); `Ledger::check_assign_role` (B-23); the projection
(schema 15) keeps no document figure; the viewer reads what a document
owes and whether it is late from `doc_state` (as of today, or of its own
date when it is dated ahead).
**Checked:** `ledger-core` `tests/spec_documents.rs` (a test per
scenario and finding of its reviews; proptests
`subledger_equals_control`, `aging_partitions_what_documents_owe`,
`reversal_twice_restores_state`); `ledger-store`
`tests/arap_standing.rs` (the fixture an earlier version wrote, its
golden figures adjudicated; the feature gate; replay equals live),
`tests/arap_props.rs` (proptest `status_is_a_function_of_journal_and_date`:
live, replayed and snapshot books agree at every date, closed months
included), the store fuzz test's voids and payments with details;
Kani `bounds_check_matches_definition`, `bounds_kept_is_the_definition_on_books_in_bounds`;
Lean `Documents.control_agrees`, `Documents.Step.document_unattached`,
`Documents.Step.reverse_moves`, `Documents.Reachable.unattached_zero`
(through role assignments and releases),
`Documents.Reachable.control_is_documents`. The model's limits: it
starts from empty books (legacy lines are covered by the identity
`Documents.control_agrees` alone), it names documents by one id space (an injective
coding of `DocRef`), and an account becoming a control account by its
first document, holding lines from before, is outside it (those lines are
listed as unattached).

### Product rules

**WB-arap-1** The tag lives beside the line, like its cleared flag; the
line and its hash never change (`BS-ledger-4`). The document commands
write it in the step that posts the line. Replay derives it from the
document events already journaled: an invoice's or bill's own entry gets
its one line on the role's account as the role stood at that point of the
journal; a payment gets its first untagged control line for the payment's
amount, so several payment events naming one entry tag its lines in
order. Each is tagged on the side its movement posts (an origin on the
owed side, a payment on the other), so an older payment deposited into
receivables itself pays with its credit. The projection (schema 15) keeps
no document figure.
**Enforced:** `Ledger::{line_doc, doc_lines}`, `replay_invoice`,
`replay_bill`, `replay_invoice_payment_at`, `replay_bill_payment_at`; the
projection keeps no document figure. **Checked:**
`ledger-store` `tests/arap_standing.rs` (the fixture an earlier version
wrote; replay equals live), `tests/arap_props.rs`.

**WB-arap-2** The default codes. A control account is the account the
role is assigned to, or one holding a line of one of the role's
documents: the role's default code (1200 receivables, 2000 payables) once
a document posted there, or an account the role moved off. A chart's own
account at a default code (1200 Equipment, a card at 2000) takes hand
lines until a document posts there; the lines it held before are listed
as unattached from then on. The deposit or payer account of a payment
holds no role: a control account (above) holds its role, and another role
is held by its assigned account, or by the account at its code of its
type.
**Enforced:** `Ledger::{control_role_of, control_check,
check_assign_role}`, the guard in `add_line`, `add_line_fx`, `post_entry`
and `register_recurring`. **Checked:** `ledger-core`
`tests/spec_documents.rs` (the control-guard and legacy-line tests);
`ledger-store` `tests/arap_standing.rs`.

**WB-arap-3** A void is described "Void invoice 7: reason" (or the bill's
number); it keeps its id and number.
**Enforced:** `Ledger::void_document`, `Store::void_document`.
**Checked:** `ledger-core` `tests/spec_documents.rs` (the void tests);
`ledger-cli` `tests/documents.rs`.

**WB-arap-4** Journal feature `document_standing` (B-27). A
payment accepted only because an earlier payment on its document was
reversed needs it: an older version, which adds every payment event to the
document's paid amount (its line tagged or not), would show it paid above
its total. Everything else is journaled as before, and a void is the
`entry.reversed` it is.
**Enforced:** `Store::{receive_payment_with, pay_bill_with,
void_document}` (feature `document_standing`). **Checked:** `ledger-store`
`tests/arap_standing.rs` (the feature gate).

**WB-arap-5** Every refusal leaves the books and the journal unchanged,
and never panics.
**Enforced:** the checks of B-38 (`Ledger::{check_reversal,
check_void_document}`, the control guard). **Checked:**
`ledger-store` the store fuzz test's voids and payments with details;
`ledger-core` `tests/spec_documents.rs`.

## B-39 Terms give one due date and one discount date, exactly

**bookspec:** `BS-arap-3`

**Claims:** yes

**Enforced:** `ledger_core::Terms` (`new`, `parse`, `due_date`,
`discount_by`, `describe`), `kernel::discount_max`, `kernel::line_amount`,
`Ledger::check_document_line_account`.
**Checked:** `ledger-core` `tests/spec_documents.rs`
(`terms_derive_due_dates`, `discount_bounds_and_late_reason`,
`terms_describe_and_parse_round_trip`, `qty_times_price_must_equal_amount`,
`document_line_account_refuses_roles_cash_and_placeholders`,
`bill_lines_post_to_their_accounts`); Kani `discount_max_is_bounded`,
`line_amount_is_exact` and their bridge properties; over the wire
`crates/ledger-cli/tests/documents.rs` (`terms_fall_back_in_order`,
`a_bill_with_its_vendor_invoice_and_terms`, `a_line_account_must_be_allowed`).

### Product rules

**WB-arap-6** Net days are 0 to 3,650 (`calendar::check` refuses a date
past the calendar). The terms a document takes fall back in order: the
settings `invoicing.default_terms` and a party's terms per side. Terms are
stored in the store behind the feature `documents_v2`.
**Enforced:** `ledger_core::Terms::new`, `calendar::check`, the settings
`invoicing.default_terms` and a party's terms per side; in the store
behind the feature `documents_v2`. **Checked:**
`terms_derive_due_dates`; over the wire `crates/ledger-cli/tests/documents.rs`
(`terms_fall_back_in_order`); `ledger-store` `tests/arap_standing.rs`
(`documents_v2_waits_for_the_upgrade`).

## B-40 An invoice number names one invoice forever; the copy sent is kept; a vendor invoice is entered once

**bookspec:** `BS-arap-4`

**Claims:** yes

**Pick:** a vendor's invoice reference is compared after the normalization that drops case and spaces (`Ledger::check_vendor_ref`); the old text names no separate decision for it.
**Pick (BS-arap-10):** `ignore-case-and-spaces`; a vendor's invoice reference is compared after the normalization that drops case and spaces.

**Enforced:** `Ledger::next_invoice_number`, `Ledger::check_vendor_ref`,
`PartySnapshot::from_party` (the snapshots at issue, D-55);
`Store::create_invoice_issued` (feature `documents_v2`);
`ledger_package::{issued_invoice_document,
draft_invoice_document, invoice_document}` and
`ledger_import::invoice_pdf` (deterministic bytes, every string data,
every amount through `amount_parts`).
**Checked:** `ledger-core` `tests/spec_documents.rs`
(`numbers_run_and_voids_keep_theirs`, `number_widens_past_9999`,
`duplicate_vendor_ref_refused_unless_a_reason_is_given`); `ledger-store`
`tests/arap_standing.rs` (`documents_v2_waits_for_the_upgrade`,
`issued_pdf_is_filed_with_the_invoice_or_nothing_is`); over the wire
`crates/ledger-cli/tests/documents.rs` (`an_invoice_from_issue_to_paid`,
`duplicate_vendor_invoice_refused`); the page `ledger-import`
`tests/spec_import.rs::invoice_pdf_prints_what_it_should` and the unit tests
`labels_are_data`, `an_invoice_renders_deterministically`,
`two_hundred_lines_paginate`; the viewer
`a_numbered_invoice_shows_its_number_terms_bill_to_and_po` and
`the_viewer_holds_a_numbered_invoice_as_the_store_does`.

### Product rules

**WB-arap-7** Numbers are written `INV-` and four digits (`INV-0001`),
wider past 9,999 and never truncated.
**Enforced:** `Ledger::next_invoice_number`. **Checked:**
`numbers_run_and_voids_keep_theirs`, `number_widens_past_9999`.

**WB-arap-8** The issued PDF is filed in the issuing call as
`receipt.attached` on the entry and linked to the invoice; its sha256 is
in that event. The PDF is rendered before anything is journaled, so a
Typst failure changes nothing. The copy a later look prints is re-created
from the books as of a day, says so (that day, never the wall clock), and
never replaces the filed copy.
**Enforced:** `Store::create_invoice_issued` (feature `documents_v2`),
`ledger_package::{issued_invoice_document, draft_invoice_document,
invoice_document}`, `ledger_import::invoice_pdf`. **Checked:** `ledger-store`
`tests/arap_standing.rs` (`issued_pdf_is_filed_with_the_invoice_or_nothing_is`),
`ledger-import` `tests/spec_import.rs::invoice_pdf_prints_what_it_should`.

**WB-arap-9** A duplicate vendor invoice is allowed only with a
`duplicate_reason`, which is journaled with the bill.
**Enforced:** `Ledger::check_vendor_ref`. **Checked:**
`duplicate_vendor_ref_refused_unless_a_reason_is_given`; over the wire
`duplicate_vendor_invoice_refused`.

**WB-arap-10** The page prints the number the preview showed; a number
another session took in between is refused as a duplicate.
**Enforced:** `Ledger::next_invoice_number`, `Store::create_invoice_issued`.
**Checked:** `ledger-cli` `tests/documents.rs` (`an_invoice_from_issue_to_paid`);
the viewer `a_numbered_invoice_shows_its_number_terms_bill_to_and_po`.

## B-41 A debt that will not be collected is written off, and a recovery is income

**bookspec:** `BS-arap-5`

**Claims:** yes

**Pick:** `direct-write-off`: CHOICE (the owner, 2026-10-23, plan 8.3.1 A-5: bad debts by the direct write-off method; and 8.3.2: for a small company's receivables a CPA accepts the direct method and notes it, where GAAP prefers an allowance under ASC 326-20). A recovery is not a new movement (plan D-49): the same year, the write-off is reversed; a later year, a plain entry to a bad-debt-recoveries revenue account.
**Pick (BS-arap-5):** `direct-write-off`, the owner, 2026-10-23.

**Pick (BS-arap-11):** `reverse-then-plain-entry`, plan D-49.

**Enforced:** `ledger_core::Ledger::write_off` (and its replay
`replay_invoice_write_off`), `LedgerError::{WriteOffOnCashBasis,
InvalidWriteOff}`; `ledger_store::Store::write_off_invoice` (feature
`documents_v2`), `WriteOffEvidence`, `StoreError::WriteOffNeedsEvidence`;
`books.write_off_invoice` (`crates/ledger-mcp/src/tools/write_offs.rs`);
the viewer's timeline (`Movement::WriteOff`); Needs attention's write-off
item.
**Checked:** `ledger-core` `tests/spec_documents.rs`
(`half_paid_rest_written_off_is_written_off`,
`write_off_refused_on_cash_basis`, `write_off_above_outstanding_refused`,
`write_off_then_recovery_same_year`); `ledger-store`
`tests/arap_standing.rs` (`write_off_waits_for_documents_v2`,
`write_off_replays_and_projects`, `write_off_without_evidence_is_refused`),
`tests/arap_props.rs` (`status_is_a_function_of_journal_and_date` with
write-offs); over the wire `crates/ledger-cli/tests/documents.rs`
(`write_off_then_recovery`, `write_off_needs_evidence`); bookspec
`bad-debt-direct-write-off`; Lean `Documents.Step.writeOff_unattached`,
`Documents.writeOff_within_bounds`; the viewer
`a_write_off_shows_on_the_timeline`; the item
`a_write_off_is_flagged_for_the_cpa`.

### Product rules

**WB-arap-11** The bad-debt expense is the `bad_debts` control role
(`BAD_DEBTS`, account 6990). The write-off's movement is `WriteOff`. There
is no `Movement::Recovery` and no bad-debt-recoveries role: a recovery is
the plain entry the record describes, or the reversed write-off.
**Enforced:** `ledger_core::Ledger::write_off`, the `bad_debts` role
(`BAD_DEBTS`, 6990). **Checked:** `half_paid_rest_written_off_is_written_off`,
`write_off_then_recovery_same_year`.

**WB-arap-12** The evidence is filed in the same call: a
`receipt.attached` on the write-off's entry, linked to the invoice. A
write-off with no evidence is refused.
**Enforced:** `ledger_store::Store::write_off_invoice`, `WriteOffEvidence`,
`StoreError::WriteOffNeedsEvidence`, `books.write_off_invoice`.
**Checked:** `ledger-store` `tests/arap_standing.rs`
(`write_off_without_evidence_is_refused`); over the wire `write_off_needs_evidence`.

**WB-arap-13** The books flag each write-off made this year for the CPA's
note (Needs attention); the tax judgments stay with the CPA.
**Enforced:** Needs attention's write-off item. **Checked:**
`a_write_off_is_flagged_for_the_cpa`.

**WB-arap-14** `invoice.written_off {invoice_id, entry_id, date, amount,
reason}` is a new event type and needs journal feature `documents_v2`
(B-27, D-52): an older version refuses it rather than misreading a
document as still owed. Replay tags the receivable line `WriteOff`; the
projection keeps no document figure (`BS-arap-2`). A refused write-off
leaves the books and the journal unchanged and never panics.
**Enforced:** `replay_invoice_write_off`, `Store::write_off_invoice`.
**Checked:** `ledger-store` `tests/arap_standing.rs`
(`write_off_waits_for_documents_v2`, `write_off_replays_and_projects`).

## B-42 A settlement is exact and names its lines

**bookspec:** `BS-arap-6`

**Claims:** yes

**Pick:** the gross method for an early-payment discount (the owner, A-8); the processor's fee posted gross (the owner, A-4); D-49's refusal to apportion a discount across a bill's asset lines. ASC 606-10-32-5 to 32-9 are the strict treatment of an expected discount; this company's size makes the gross method immaterial (A-8). The sources are cited and not yet vendored (the Open list in `README.md`).
**Pick (BS-arap-12):** `gross-when-taken`, the owner, A-8.

**Pick (BS-arap-13):** `gross`, the owner, A-4.

**Pick (BS-arap-14):** `refuse-with-the-remedy`, plan D-49.

**Enforced:** `kernel::settle`, `Ledger::{settle, settlement_lines,
check_discount}`, `arap::{Application, Fee, SettleParams, SettlementRow}`,
`Ledger::{replay_invoice_payment_at, replay_bill_payment_at}`,
`Store::settle`, the aging builders (`Ledger::aging_documents`,
`ledger_package::{aging_csv, aging_statement}`). **Checked:** `ledger-core` `tests/spec_documents.rs`
(`bill_on_terms_with_discount`, `late_discount_needs_a_reason`,
`discount_on_an_asset_bill_is_refused_with_the_remedy`,
`a_fee_posts_gross_to_receivables_and_net_to_the_bank`,
`a_settlement_names_its_lines_and_refuses_a_document_twice`),
`tests/kernel_bridge/main.rs::settle_agrees`; Kani `settle_is_exact`;
`ledger-store` `tests/arap_standing.rs`
(`a_settlement_names_its_lines_and_replays`, `settlement_waits_for_documents_v2`,
`legacy_payment_events_keep_the_positional_rule`), `tests/arap_props.rs`;
`ledger-cli` `tests/documents.rs`; `ledger-package`'s aging tie-out test.
The Lean step is the document constructor of the Lean documents model, with
every control line tagged (`Documents.settle_unattached`); no new step
constructor.

### Product rules

**WB-arap-15** The entry's rows are the payment lines in application
order, then the discount lines, then the fee line, then the cash line. A
new payment event names the line it tags (`line`) and, when it took one,
its discount and discount line (`discount`, `discount_line`,
`discount_reason`); the fee's are on the first application's event (`fee`,
`fee_line`). Replay tags exactly those lines; a legacy event with no
`line` takes the first untagged control line for its amount
(`BS-arap-2`, `WB-arap-1`). A line replay cannot match stays untagged and
the control check lists it: replay never refuses (`BS-entry-4`).
**Enforced:** `Ledger::{settle, settlement_lines,
replay_invoice_payment_at, replay_bill_payment_at}`, `Store::settle`.
**Checked:** `ledger-store` `tests/arap_standing.rs`
(`a_settlement_names_its_lines_and_replays`,
`legacy_payment_events_keep_the_positional_rule`);
`a_settlement_names_its_lines_and_refuses_a_document_twice`.

**WB-arap-16** The aging CSV (one row per open document, then the
tie-out line) is written the same by the CPA package and by
`books.export`.
**Enforced:** `ledger_package::{aging_csv, aging_statement}`,
`Ledger::aging_documents`. **Checked:** `ledger-package`'s aging tie-out
test.

**WB-arap-17** The refusal of a discount on a bill with an asset or
prepaid line names D-49's remedy in accounts: record the bill net of the
discount, or reclassify Dr 6995 Purchase Discounts / Cr the asset.
**Enforced:** `Ledger::check_discount`, `kernel::settle`. **Checked:**
`discount_on_an_asset_bill_is_refused_with_the_remedy`,
`a_fee_posts_gross_to_receivables_and_net_to_the_bank`.
