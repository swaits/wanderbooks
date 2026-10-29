# Invoices and bills

An invoice bills a customer: it posts Dr Accounts Receivable and Cr the
revenue its lines name. A bill is a vendor's invoice to us: it posts Dr
the expense or asset its lines name, and Cr Accounts Payable. A document's
own figures are read from the ledger lines tagged with it, never kept on
the document: a reversed payment, a void or a write-off comes out right
without another copy to correct (B-38).

{documents_v2}

## 1. Issue and send an invoice

- Preview first: `books.create_invoice` without `confirmed` writes
  nothing. It returns the posting, the number and due date the invoice
  will take, and its DRAFT PDF; show the human the PDF and the posting.
  An abandoned preview uses up no number.
- Confirm with `confirmed: true` only after the human has seen it. The
  invoice posts Dr Accounts Receivable / Cr each line's account, takes the
  next number (B-40), and the issued PDF, the page the customer gets, is
  filed with it. That filed copy never changes: a later look at the
  invoice (`books.document` with `pdf: true`, or the viewer's Save invoice
  PDF) is a re-created copy as of that day, says so, and shows what has
  been paid since.
- Give a line a quantity and a unit price when the work is billed that way
  (`quantity` times `unit_price` must equal the line's amount exactly); the
  page prints "3 × $33.33" under its description. The customer's purchase
  order goes in `customer_ref`; `memo` prints under the lines. The
  company's email, website, payment methods (each can show a QR code) and
  footer come from the settings, not from the call.
- The page prints the customer as it was when the invoice was issued (its
  name, billing address, contact, phone and billing email, and a Ship-to
  line when it has a shipping address); changing the customer's record
  later never changes an issued invoice.
- The books never send mail, and never say they did. Hand the issued PDF
  to the human (`books.document` with `pdf: true`; the viewer's Save
  issued copy), and let the owner or a delegate send it.
- Then record the send with `books.mark_sent`: the invoice (`invoice:7`),
  the day it went out, who sent it (`by`: the owner, or the agent acting
  for them), how (`via`: `email`, or `mail`, `portal`, `hand`, or another
  channel's own name), to whom (`to`: the bill-to email, else the
  customer's email), whether it was the original or a reminder (`kind`:
  `original` for the first send, `reminder` after), and the channel's own
  reference when there is one (a message id).
- A send changes no figure and never moves the due date. Several sends
  are allowed: each is recorded in order, and `books.documents` shows the
  latest as `last_sent`.
- A void invoice can't be marked sent: it should never have reached the
  customer. An invoice already open, partly paid, paid or written off can
  (a copy for the customer's records, a reminder).

## 2. Record a customer payment

- Ask for the date the money arrived, where it landed, and what it paid.
  One deposit may settle several invoices of the same customer: pass
  `applications` with each invoice and its cash amount.
- Record how it moved (`instrument`) and the processor's or bank's
  reference. The books record payments; they never collect them.
- If the customer paid more than the invoices owe, the payment is
  refused: record the excess as a plain entry to 2300 Customer Deposits
  (no role) and apply it when the work is invoiced (D-49, A-12).
- A check or note marked "paid in full" of a disputed amount earns the
  UCC §3-311 warning. Record only the payment; a lawyer or the payer's
  agreement settles the rest.

## 3. Pay a vendor bill

- Ask for the date, the account it left, and which bills it pays. One
  payment may settle several bills of one vendor (`applications`).
- Record the method and the reference. A card payment is tagged as card
  and stays out of 1099-NEC (D-40).
- Use `books.bills_due` for what is due and when, with each vendor's
  early-payment discount window.

### Taking an early-payment discount

- A bill on "2/10 net 30" takes 2% off if paid within ten days. Pass the
  discount beside the cash (`discount`); the books post the gross method:
  the bill is credited in full, the discount goes to Purchase Discounts
  (6995), and the bank gets the rest.
- A discount at most the terms allow and dated on or before their
  discount date needs no reason. A larger or later one needs
  `discount_reason` (a vendor who honours it anyway); it is journaled.
- A bill with an asset or prepaid line refuses a discount: record the
  bill net of the discount, or reclassify with an entry Dr 6995 Purchase
  Discounts / Cr the asset account (D-49). Never split the discount across
  the asset lines.

## 4. Void, write off, or reverse

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

## 5. A processor's fee

- The books never charge or collect; a processor does, and keeps a fee.
  Pass `fee` on `books.receive_payment` (amount, and optionally the
  expense account and the processor's vendor).
- The entry is gross: Dr the deposit account the net, Dr the
  processing-fee expense the fee, Cr Accounts Receivable the gross (A-4).
  Only cash movements count toward a 1099, so the fee is not a payment to
  the processor.

## A replacement payment arrives before a bounce is known

1. Park it: a plain entry Dr the bank / Cr 2300 Customer Deposits (no
   role), described with the customer and what it is for.
2. When the first payment bounces, reverse it (`books.reverse_entry`)
   dated the bounce day; it un-pays its invoices.
3. Apply the parked money with `books.receive_payment`,
   `deposit_to: "2300"`, dated on or after the bounce, against the
   invoices it pays. The 2300 balance returns to zero.

## 6. What the CPA will check

- The agings tie to 1200 and 2000 at month and year end (they do by
  construction: `books.control_check` says so; run it at month end and
  tie the aging to the trial balance before the CPA asks).
- Invoice numbers are unique and sequential, voids stay visible with
  their number, nothing is deleted.
- Every bill is coded to its real account (the Form 1065 lines, B-32),
  never a catch-all, with the vendor's invoice attached and its number
  recorded, and no bill is paid twice (the same vendor and number twice is
  refused).
- Payments are matched to documents, and card payments are identified
  (excluded from 1099-NEC: Treas. Reg. §1.6041-1(a)(1)(iv)).
- Customer deposits are liabilities (2300), never netted inside
  receivables; Schedule L lines 2a and 15 come straight from the control
  accounts.
- Approvals and three-way matching are deliberately not adopted for a
  two-member LLC: light governance, by decision.
- Every write-off carries its reason and its evidence of collection
  efforts. Write-offs are not used in
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
