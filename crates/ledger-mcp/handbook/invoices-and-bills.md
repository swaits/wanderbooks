# Invoices and bills

An invoice is money owed to you (Accounts Receivable, 1200); a bill is money
you owe (Accounts Payable, 2000). Each is a document whose figures come from
the ledger lines tagged with it: what is paid, discounted, written off and
still owed is read from those lines, never kept beside them, so a reversal,
a void or a bounced payment is right by construction. `books.documents`
lists them, `books.document` shows one with its timeline, and
`books.control_check` proves the control accounts equal their documents.

## Step 4: void, write off, or reverse

One of these, and only one, fits what happened:

- **Void it** (`books.void_document`) when the document should never have
  existed: a duplicate, the wrong customer or vendor, a test entry. It is
  reversed, keeps its id (and, once numbered, its number) and reads void;
  nothing is deleted. Refused while a payment stands on it: reverse that
  payment first, and the refusal names it. Use a void for a mistake, never
  for an invoice that was real.
- **Write it off** (`books.write_off_invoice`) when the invoice was real
  and the work or goods went out, but the customer will not or cannot pay.
  That is a bad debt: the books post Dr Bad Debt Expense 6990 / Cr Accounts
  Receivable for what will not be collected, and what it owes falls from
  that day. The evidence of worthlessness — the collection letters, the
  bankruptcy or liquidation notice, the collection agency's report — is
  filed with the write-off in the same call (`upload_id`, or an
  `existing_attachment_id` already filed), because a bad debt is deducted
  on all the pertinent evidence (Treas. Reg. §1.166-2(a)). Write off all of
  it, or part of it with `amount`: a partial write-off is deductible to the
  extent charged off (Treas. Reg. §1.166-3(a)), and the rest stays owed and
  in the aging. The books must be on the accrual basis: a cash-basis
  company never counted the invoice as income, so there is no basis to
  deduct (IRC §166; Treas. Reg. §1.166-1(e)), and the tool refuses.
- **Reverse an entry** (`books.reverse_entry`) when a posted entry is
  wrong: a payment against the wrong invoice, a wrong amount, a check that
  bounced, a write-off made too soon. Posted is forever: the correction is
  an opposite entry linked to the one it reverses, never an edit and never
  a delete. Reversing a payment puts the invoice back to owing what it did;
  reversing a write-off re-instates the debt.

A bill is never written off: a vendor releases you with a vendor credit or
by voiding its own invoice, and until then you owe it (ASC 405-20-40-1). If
a vendor's bill was wrong, ask for a corrected bill or record a credit when
one is built; void it only when it should never have been entered.

## Recovering a debt written off

- **The same tax year**: reverse the write-off — `books.reverse_entry` on
  the write-off's entry. The invoice reopens, owing exactly what the
  write-off took off; then record the payment with `books.receive_payment`
  as usual. Nothing else is needed: within the year the deduction comes
  back out, and the payment is a payment.
- **A later tax year**: do not reverse the write-off. Record a plain entry
  — `books.draft_entry`, `books.add_line` on each side, `books.post_entry`
  — debiting the deposit account and crediting a "Bad debt recoveries"
  revenue account (tax category other income), with a memo naming the
  invoice. The invoice stays written off, and the recovery is income in the
  year it arrives (IRC §111); the CPA sees it apart from the year's sales.

Neither procedure invents a new kind of movement on the document: the books
record what happened, and the invoice's timeline shows the write-off (and,
if it was reversed, that it no longer stands).

## What the CPA will check

The agings tie to 1200 and 2000 at month and year end (they do by
construction: the control check says so). Every write-off carries its
reason and its evidence of collection efforts. Write-offs are not used in
place of a credit memo for a valid invoice, and never to hide a mistake
that a void would show. Whether a debt is worthless, and in which year, is
a tax judgment that stays with the CPA: the books flag each write-off made
this year in Needs attention, with the evidence beside it, until the year
ends.

## The evidence rule

Every entry needs its receipt ({evidence_rule}). A document the books post
themselves — an invoice, a bill, a payment, a write-off — carries its own:
the issued copy, the vendor's bill, the payment's reference, the evidence of
worthlessness. A reversal needs none of its own: its original carries it.
