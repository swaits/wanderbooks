# Invoices and bills

Invoices are what customers owe the company; bills are what it owes
vendors. Every figure on them is read from the ledger lines tagged with
them, so a reversal, a void or a bounced payment is right by
construction: never edit one.

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
