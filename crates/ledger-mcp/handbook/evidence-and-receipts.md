# Evidence and receipts

- {evidence_rule}
- Depreciation and amortization are exempt: their schedules are the evidence. A period close is exempt too: its closing entry is derived from the books themselves. A reversal relies on its original's receipt.
- Not receipt-checked yet: books.create_invoice, books.create_bill, books.receive_payment, books.pay_bill, books.run_payroll and books.remit_payroll_taxes post at once, with no preview and no receipt check. Attach their evidence (the invoice, the bill, the payment confirmation, the payroll register) right after, linked to the entry they return.
- Receipt corrections: receipts can change, but nothing is erased. To replace one, attach the new receipt, then books.unlink_receipt the old (with a reason); retire unwanted receipts or documents with books.withdraw_attachment (with a reason). The journal keeps every step.
- File a receipt once and link it wherever it is evidence (the attachments section).
