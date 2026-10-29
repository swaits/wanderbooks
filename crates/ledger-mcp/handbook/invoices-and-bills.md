# Invoices and bills

An invoice bills a customer: it posts Dr Accounts Receivable and Cr the
revenue its lines name. A bill is a vendor's invoice to us: it posts Dr
the expense or asset its lines name, and Cr Accounts Payable. A document's
own figures are read from the ledger lines tagged with it, never kept on
the document: a reversed payment, a void or a write-off comes out right
without another copy to correct (B-38).

## Sending an invoice

- The books never send mail, and never say they did. To send an invoice,
  create and post it (`books.create_invoice`), hand its PDF to the human
  (`books.document` with `pdf: true`), and let the owner or a delegate
  send it.
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
