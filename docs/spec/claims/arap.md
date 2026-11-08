# Claims: arap

Wanderbooks' claims for bookspec area **arap**: what each rule requires, where the engine enforces it, and which tests check it. Being reduced to the engine's pick and its evidence as P14 unit S-D lands; the accounting text moves to bookspec's `spec/arap.md`.

Rules: B-18, B-38, B-39, B-40, B-41, B-42.

---

## B-18 Aging buckets: Current, 1–30, 31–60, 61–90, 90+ days past due

**bookspec:** `BS-arap-1`

**Status:** CONVENTION (bucket edges); CHOICE (due today is Current)

ASC 326-20-55-37/38 (cited) illustrates "one way" to age trade
receivables with buckets "current … 1–30 days past due … 31–60 … 61–90 …
more than 90 days past due"; it is illustrative, not a rule. The FFIEC
Call Report instructions (bank regulators, not binding here): "The past
due status … should be determined in accordance with its contractual
repayment terms." A document due today is not yet past due: CHOICE
(the owner, 2026-10-13). Aging as of a date shows only documents issued by
then, less only payments made by then; and a payment never predates its
invoice or bill (a real prepayment is a deposit): CHOICES (the owner,
2026-10-13). What a document owes as of a date is read from its ledger
lines that stand then (B-38): a void invoice is in no bucket, a payment
reversed by then no longer counts, one reversed later still does; the
buckets partition what the documents owe, exactly.

**Enforced:** `arap::aging_bucket`, `Ledger::ar_aging`/`ap_aging`
(through `Ledger::doc_state`), `receive_payment`/`pay_bill`
(`PaymentBeforeDocument`). **Checked:**
`aging_boundaries`, `aging_is_exclusive_and_only_ages`,
`aging_as_of_a_date_sees_only_what_existed_then`,
`a_payment_never_predates_its_document`, `aging_as_of_ignores_a_later_reversal`,
`a_voided_invoice_never_ages`, `status_is_a_function_of_journal_and_date`.

## B-38 A document's figures are its ledger lines that stand; a control account is its documents

**bookspec:** `BS-arap-2`

**Status:** CHOICE (the owner, 2026-10-23, plan D-45: "we kinda need
stronger invoice support. Like make it a first class citizen"; hand
entries to receivables and payables refused, the owner's answer 5);
CONVENTION (a control account agrees with its subledger, as B-23; the
subledger tie-out is the first thing a CPA checks); sources cited and not
yet vendored, so this rule keeps the CONVENTION label (the Open list in
`README.md`): ASC 310-10 (receivables), ASC 405-20-40-1 (a liability is
removed only when paid or released), ASC 606-10-45-4 (a receivable is a
right to consideration that is unconditional).

1. **Tags.** Every ledger line on the receivables or payables control
   account carries the document it belongs to (an invoice or a bill) and
   what it does to it: its origin, a payment (later: a discount, a
   write-off, an adoption, a role move). The tag lives beside the line,
   like its cleared flag; the line and its hash never change (B-8). The
   document commands write it in the step that posts the line; replay
   derives it from the document events already journaled (an invoice's
   or bill's own entry: its one line on the role's account as the role
   stood at that point of the journal; a payment: its first untagged
   control line for the payment's amount, so several payment events
   naming one entry tag its lines in order), each on the side its
   movement posts (an origin on the owed side, a payment on the other, so
   an older payment deposited into receivables itself pays with its
   credit); a reversal's mirror line carries its original's.
2. **Figures.** What a document owes as of a day is the signed sum of
   the lines tagged with it dated on or before that day (a debit to
   receivables counts plus for an invoice, a credit to payables for a
   bill); what it was paid is minus the same sum over its payment lines.
   Its status: Void when its own entry doesn't stand on that day (B-9;
   an entry stands while an even number of the reversals after it in its
   chain are dated by then, counted in chain order: a reversal counts
   only once what it reverses does), else Unattached when its own entry's
   control line carries no tag (an older entry replay couldn't match),
   else Overpaid when it owes less than nothing (a credit balance, owed
   back: never a receivable or payable, ASC 310-10), else Written off,
   Paid, Partly paid or Open by what it owes. Overdue is a flag (owed past
   the due date), not a status. No figure is stored: a reversal, a void
   or a bounced payment is right by construction, and the aging as of a
   day (B-18) shows what was owed then; it leaves out a document owing
   less than nothing, which the control check lists with the unattached
   ones for attention.
3. **The control guard.** A control account is one the receivables or
   payables role is assigned to, or one holding a line of one of the
   role's documents (the role's default code once a document posted
   there; an account the role moved off). A line on one that carries no
   document is refused, when added and when posted, whatever writes it
   (a hand entry, a template, a recurring entry); a recurring template
   naming one is refused when registered. The one exception is the
   mirror of an untagged line in a reversal that takes the line off the
   books (the hand line itself, or a re-instatement of it, reversed):
   reversing an old hand line is how it is cleaned up, and reversing that
   cleanup, which would put it back, is refused. The role never moves
   onto an account holding an untagged line (B-23). Replay never refuses
   (B-28). A chart's own account at a default code (1200 Equipment, a
   card at 2000) takes hand lines until a document posts there; lines it
   held before are listed as unattached from then on.
4. **Bounds.** At every day, 0 ≤ owed ≤ total, for every document
   written under these rules. A payment can't exceed what is owed on any
   day from its date on; a reversal that would break the bound where it
   held (a void under a payment that stands; re-instating a payment
   beside its replacement) is refused, naming what to reverse first;
   reversing a payment is always taken. Books an earlier version left out
   of bounds take any step that makes no day worse, so the remedy is
   accepted (a step may carry a day across the range, overpaid to
   over-owed, so long as it lands nearer the range). A reversal of a
   document's entry is never dated before that entry.
5. **Payments.** The deposit or payer account is a postable asset or
   liability that holds no role: a bank, a card, a member's payable. A
   control account (item 3) holds its role; another role is held by its
   assigned account, or by the account at its code of its type.
   The payment method and the processor's reference go on the payment's
   entry in the same step (B-19, D-40).
6. **Voids.** Voiding a document reverses its own entry, described
   "Void invoice 7: reason"; it keeps its id (and number), and reversing
   the void re-instates it (B-10).
7. **Journal.** A payment accepted only because an earlier payment on
   its document was reversed needs the journal feature
   `document_standing` (B-27): an older version, which adds every
   payment event to the document's paid amount (its line tagged or not),
   would show it paid above its total. Everything else is journaled as before, and a void is the
   `entry.reversed` it is.

```text
GOAL: invoices and bills are the subledgers of the receivables and
      payables control accounts; every figure about a document is read
      from the ledger lines tagged with it.
INVARIANTS:
  1. For every control account A and day D:
     GL(A, D) = Σ_docs outstanding(d, D) + unattached(A, D).
     No command adds an untagged control line; unattached changes only by
     reversing (or, later, adopting) a legacy line.
  2. 0 ≤ outstanding(d, D) ≤ total(d) at every D for every document
     written under these rules; replay accepts what was written.
  3. Reversing a tagged entry changes outstanding by exactly the opposite
     of its tagged lines, from the reversal's day; reversing that
     reversal restores it.
  4. A document's state is a function of (the journal, the day).
  5. Aging as of D partitions Σ outstanding(·, D) over the documents
     owing something; a credit balance (only in books an earlier version
     left out of bounds) is left out and listed by the control check.
  6. Every refusal leaves the books and the journal unchanged; NEVER panics.
SCENARIOS:
  1. Invoice 100, payment 100, payment reversed: Open, owing 100, A/R 100.
  2. Invoice reversed with nothing paid: Void, not in aging, id kept.
  3. Void with a payment standing: SHOULD refuse, naming the payment.
  4. As of a day between a payment and its reversal: counted as paid.
  5. Wrong payment reversed, right one recorded: accepted (feature
     document_standing declared first).
  6. Hand Dr 1200 / Cr 4000: SHOULD refuse with the document tools
     named; reversing a legacy hand line on 1200: accepted, unattached
     shrinks.
  7. (S7) Half paid, rest written off: Written off, 50 paid / 50 written off.
  8. receive_payment into 1200 or 4000: SHOULD refuse.
```

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
`control_agrees` alone), it names documents by one id space (an injective
coding of `DocRef`), and an account becoming a control account by its
first document, holding lines from before, is outside it (those lines are
listed as unattached).

## B-39 Terms give one due date and one discount date, exactly

**bookspec:** `BS-arap-3`

**Status:** CONVENTION (standard trade-terms notation: "Net 30", "Net 15
EOM", "2/10 net 30", "Due on receipt"). Sources cited and not yet
vendored, so this rule keeps the CONVENTION label (the Open list in
`README.md`): UCC §2-310(a) (for goods with no agreed terms, payment is
due when the buyer receives them); for services, the contract sets the
terms. Sending an invoice late never moves its due date.

```text
GOAL: terms give one due date and one discount date, exactly.
SOURCE: CONVENTION: standard trade-terms notation; UCC §2-310(a) for
        goods; for services, the contract.
INVARIANTS:
  1. Net n = date + n; Net n EOM = last day of date's month + n; due on
     receipt = date; Custom = the given date. Always at least date, in the
     calendar (calendar::check); n in 0..=3650.
  2. discount_by = date + d, d below the net days (none for due on
     receipt or EOM); a taken discount is at most
     kernel::discount_max(total, percent_bp), the caller's exact cents;
     after discount_by only with a reason.
  3. The canonical spellings read back as they print.
  4. A line given as a quantity and a unit price has
     quantity x unit price = amount exactly (kernel::line_amount, neither
     rounded); a line's account is a revenue or deferred-revenue account
     on an invoice, an expense or a non-cash asset on a bill, never a role
     account, cash or a placeholder.
SCENARIOS:
  1. 2026-01-31 net 30 -> 2026-03-02; 2026-10-23 net 15 EOM ->
     2026-11-15; a leap day; due on receipt = the day.
  2. The 9999-12 bound is refused, not a panic.
```

**Enforced:** `ledger_core::Terms` (`new`, `parse`, `due_date`,
`discount_by`, `describe`), `kernel::discount_max`, `kernel::line_amount`,
`Ledger::check_document_line_account`; the settings
`invoicing.default_terms` and a party's terms per side; in the store
behind the feature `documents_v2`.
**Checked:** `ledger-core` `tests/spec_documents.rs`
(`terms_derive_due_dates`, `discount_bounds_and_late_reason`,
`terms_describe_and_parse_round_trip`, `qty_times_price_must_equal_amount`,
`document_line_account_refuses_roles_cash_and_placeholders`,
`bill_lines_post_to_their_accounts`); Kani `discount_max_is_bounded`,
`line_amount_is_exact` and their bridge properties; over the wire
`crates/ledger-cli/tests/documents.rs` (`terms_fall_back_in_order`,
`a_bill_with_its_vendor_invoice_and_terms`, `a_line_account_must_be_allowed`).

## B-40 An invoice number names one invoice forever; the copy sent is kept; a vendor invoice is entered once

**bookspec:** `BS-arap-4`

**Status:** CONVENTION (sequential invoice numbers with voids kept, and a
duplicate-payment control). Sources cited and not yet vendored, so this
rule keeps the CONVENTION label (the Open list in `README.md`): IRS Pub.
583 (invoices are among the records to keep); Rev. Proc. 98-25 (an
electronic record keeps its audit trail: why the PDF as issued is filed
with its hash and a void keeps its number).

```text
GOAL: an invoice number names one invoice forever; the copy sent is kept;
      a vendor invoice is entered once.
SOURCE: Pub. 583; Rev. Proc. 98-25; CONVENTION: sequential numbers, voids
        kept; duplicate-payment control.
INVARIANTS:
  1. Numbers are unique, in issue order, never reused; a void keeps its
     number. Four digits, wider past 9,999, never truncated.
  2. Number, snapshots, terms and lines never change after issue.
  3. The issued PDF is filed in the issuing call as receipt.attached on
     the entry and linked to the invoice; its sha256 is in that event. The
     copy a later look prints is re-created from the books as of a day,
     says so (that day, never the wall clock), and never replaces it.
  4. (vendor, normalized vendor_ref) is unique among standing bills unless
     a duplicate_reason is journaled; the normalization drops case and
     spaces.
  5. Sends never change a figure.
SCENARIOS:
  1. Three invoices, the second voided: INV-0001, INV-0002 (void), INV-0003.
  2. A second bill from the same vendor with the same number: refused
     unless a reason says why.
  3. The page prints the number the preview showed; a number another
     session took in between is refused as a duplicate.
```

**Enforced:** `Ledger::next_invoice_number`, `Ledger::check_vendor_ref`,
`PartySnapshot::from_party` (the snapshots at issue, D-55);
`Store::create_invoice_issued` (feature `documents_v2`; the PDF is
rendered before anything is journaled, so a Typst failure changes
nothing); `ledger_package::{issued_invoice_document,
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

## B-41 A debt that will not be collected is written off, and a recovery is income

**bookspec:** `BS-arap-5`

**Status:** CHOICE (the owner, 2026-10-23, plan 8.3.1 A-5: bad debts by
the **direct write-off** method, and 8.3.2: for a small company's
receivables a CPA accepts the direct method and notes it, where GAAP
prefers an allowance under ASC 326-20). AUTHORITY for what the rule
rests on: IRC §166(a) (a debt that becomes worthless is deductible in
that year), Treas. Reg. §1.166-1(e) (a debt never counted as income has
no basis to deduct), §1.166-2(a) (worthlessness on all the pertinent
evidence), §1.166-3(a) (a partial write-off, to the extent charged off),
and IRC §111 (a recovery of an amount deducted is income in the year
received). ASC 405-20-40-1 for a bill's release.

1. **The write-off.** Writing off an invoice posts one balanced entry:
   Dr the bad-debt expense role (`bad_debts`, 6990) and Cr receivables
   for the amount, the receivable line carrying the invoice with the
   movement `WriteOff`, so what the invoice owes falls by the amount from
   the write-off's day and what was written off rises by it. The amount
   is above zero and at most what the invoice owes, and at most the least
   it owes on any day from the write-off's on (so no later day, a payment
   after it included, takes the document below nothing: B-38 INVARIANT 2).
   A partial write-off is allowed (§1.166-3(a)); the rest stays owed and
   aged.
2. **The day.** The write-off's day is in the calendar, in an open period
   (B-12), and not before the invoice's. A document's entry is reversed,
   never edited (B-8, B-9): a write-off is not a void and a void is not a
   write-off.
3. **Cash basis refused.** Books on the cash basis never counted the
   invoice as income, so the debt has no basis to deduct (§1.166-1(e)):
   the command refuses, changing nothing.
4. **Bills are never written off** (ASC 405-20-40-1): a liability leaves
   the books only when paid or when the creditor releases it, which is a
   vendor credit (not scheduled). The command takes an invoice.
5. **A recovery is not a new movement** (plan D-49). In the same tax year,
   reverse the write-off: the invoice reopens and owes what the write-off
   took off, and the payment is recorded as any payment. In a later year,
   record a plain entry Dr the deposit account / Cr a "Bad debt
   recoveries" revenue account (tax category other income), its memo
   naming the invoice: the recovery is income (§111) and the invoice stays
   written off. There is no `Movement::Recovery` and no bad-debt-recoveries
   role.
6. **Evidence.** The write-off's evidence of worthlessness — collection
   correspondence, a bankruptcy or liquidation notice (§1.166-2(a)) — is
   filed in the same call: a `receipt.attached` on the write-off's entry,
   linked to the invoice. A write-off with no evidence is refused. The tax
   judgments (whether the debt is worthless, and in which year) stay with
   the CPA: the books flag each write-off made this year for the CPA's
   note.
7. **Journal.** `invoice.written_off {invoice_id, entry_id, date, amount,
   reason}` is a new event type and needs journal feature
   `documents_v2` (B-27, D-52): an older version refuses it rather than
   misreading a document as still owed. Replay tags the receivable line
   `WriteOff`; the projection keeps no document figure (B-38).

```text
GOAL: write off what will not be collected, as the tax rules require.
SOURCE: IRC §166(a), §111; Treas. Reg. §1.166-1(e), (f), -2(a), -3(a);
        ASC 326-20 and ASC 405-20-40-1 noted; CHOICE: the direct
        write-off method (plan 8.3.1 A-5).
INVARIANTS:
  1. amount <= outstanding on the write-off's day and on every day from
     it; Dr bad_debts / Cr receivables [d, WriteOff]; the evidence of
     worthlessness filed in the same call.
  2. Its reversal re-instates the debt; bills are never written off.
  3. Refused on cash-basis books.
  4. A refusal leaves the books and the journal unchanged; NEVER panics.
SCENARIOS:
  1. Half paid, the rest written off: Written off, with 50 paid / 50
     written off.
  2. A write-off of 0, a negative amount, or more than is owed: SHOULD
     refuse, naming what is owed.
  3. Cash-basis books: SHOULD refuse (no basis to deduct).
  4. The write-off reversed in the same year and the payment recorded:
     Paid; receivables back to zero.
  5. A later year's recovery: a plain entry to a bad-debt-recoveries
     revenue account; the invoice stays written off.
```

**Enforced:** `ledger_core::Ledger::write_off` (and its replay
`replay_invoice_write_off`), `LedgerError::{WriteOffOnCashBasis,
InvalidWriteOff}`, the `bad_debts` control role (`BAD_DEBTS`, 6990);
`ledger_store::Store::write_off_invoice` (feature `documents_v2`; the
evidence filed and linked in the same call), `WriteOffEvidence`,
`StoreError::WriteOffNeedsEvidence`; `books.write_off_invoice`
(`crates/ledger-mcp/src/tools/write_offs.rs`); the viewer's timeline
(`Movement::WriteOff`); Needs attention's write-off item.
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

## B-42 A settlement is exact and names its lines

**bookspec:** `BS-arap-6`

**Status:** CONVENTION (a payment that settles several documents at once is
still one balanced entry, and each control line carries its document; the
subledger tie-out is B-38's); CHOICE (the gross method for an
early-payment discount, A-8; the processor's fee posted gross, A-4; D-49's
refusal to apportion a discount across a bill's asset lines). ASC
606-10-32-5 to 32-9 are the strict treatment of an expected discount;
this company's size makes the gross method immaterial (A-8). Sources are
cited and not yet vendored, so this rule keeps the CONVENTION label (the
Open list in `README.md`).

1. **One party, one kind.** A settlement applies to one or more invoices
   of one customer, or one or more bills of one vendor, never a mix, and
   never the same document twice. Deposits: Σ cash + Σ discounts is what
   it applies to the documents; Σ cash − fee is what reaches the deposit
   account (`kernel::settle`). The arithmetic is exact: no epsilon, no
   rounding, and a fee above the cash is refused.
2. **Bounds.** Each application's cash + discount is at most what its
   document owes on every day from the settlement's date on
   (`payment_room`), so no day is overpaid. A payment is never dated
   before its document, and never in a closed period.
3. **Discounts.** A discount is at most `kernel::discount_max(total,
   percent_bp)` for the document's terms and is dated on or before
   `terms.discount_by(date)`; outside that it needs a reason, journaled
   with the payment (a vendor honouring a late discount). Terms that
   state no discount are the same: any discount needs a reason. A discount
   on a bill whose lines include an asset or prepaid account is refused
   with D-49's remedy: record the bill net of the discount, or reclassify
   Dr 6995 Purchase Discounts / Cr the asset.
4. **The fee.** A processor's fee is a customer payment's only. It posts
   gross: Dr the processing-fee expense, Dr the deposit account the net,
   Cr accounts receivable the gross. Only cash movements count toward a
   1099, so the fee is not a payment to the processor.
5. **Lines and journal.** The entry's rows are the payment lines in
   application order, then the discount lines, then the fee line, then
   the cash line; a new payment event names the line it tags (`line`) and,
   when it took one, its discount and discount line (`discount`,
   `discount_line`, `discount_reason`), and the fee's on the first
   application's event (`fee`, `fee_line`). Replay tags exactly those
   lines; a legacy event with no `line` takes the first untagged control
   line for its amount (B-38 item 1). A line replay cannot match stays
   untagged and the control check lists it: replay never refuses (B-28).
6. **Exports.** An aging as of a day lists one row per open document with
   its number, party, dates, terms, total, paid, discounted, outstanding,
   bucket and days overdue, then a tie-out line against its control
   account (B-23, B-38 INVARIANT 1). The CPA package and `books.export`
   write the same CSV.

```text
GOAL: a settlement is exact and names its lines.
SOURCE: CONVENTION; ASC 606-10-32-5 to 32-9 noted; CHOICE: the gross
        method (A-8), the fee's gross presentation (A-4).
INVARIANTS:
  1. Σ cash + Σ discounts = Σ applications; deposit = Σ cash − fee
     (kernel::settle); one party per settlement; one application per
     document.
  2. Each application's cash + discount ≤ what the document owes on every
     day from the payment's date on.
  3. Each new payment event names the line it tags; replay tags that line;
     a legacy event without one takes the first untagged control line for
     its amount.
  4. A discount on a bill with an asset or prepaid line is refused.
SCENARIOS:
  1. Two invoices of one customer, 300 and 80, paid 380 by one deposit:
     two payment lines, each tagged, one cash line.
  2. "2/10 net 30" bill 100, paid 98 within ten days: Dr A/P 98 (payment)
     and 2 (discount), Cr purchase discounts 2, Cr bank 98.
  3. The same discount after the tenth day without a reason: refused,
     naming discount_reason; with one: taken and journaled.
  4. An invoice 100 paid through a processor keeping 3: Dr bank 97, Dr
     processing fees 3, Cr A/R 100.
  5. A discount on a bill whose line is equipment: refused with the
     record-net or reclassify remedy (D-49).
```

**Enforced:** `kernel::settle`, `Ledger::{settle, settlement_lines,
check_discount}`, `arap::{Application, Fee, SettleParams, SettlementRow}`,
`Ledger::{replay_invoice_payment_at, replay_bill_payment_at}`,
`Store::settle`, the aging builders (`Ledger::aging_documents`,
`ledger_package::{aging_csv, aging_statement}`). **Checked:** `ledger-core` `tests/spec_documents.rs`
(`bill_on_terms_with_discount`, `late_discount_needs_a_reason`,
`discount_on_an_asset_bill_is_refused_with_the_remedy`,
`a_fee_posts_gross_to_receivables_and_net_to_the_bank`,
`a_settlement_names_its_lines_and_refuses_a_document_twice`),
`tests/kernel_bridge.rs::settle_agrees`; Kani `settle_is_exact`;
`ledger-store` `tests/arap_standing.rs`
(`a_settlement_names_its_lines_and_replays`, `settlement_waits_for_documents_v2`,
`legacy_payment_events_keep_the_positional_rule`), `tests/arap_props.rs`;
`ledger-cli` `tests/documents.rs`; `ledger-package`'s aging tie-out test.
The Lean step is `Documents.Step.document` with every control line tagged
(`Documents.settle_unattached`); no new `Step` constructor.

