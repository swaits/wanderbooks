# Area: arap

Receivables and payables: documents, aging, terms, write-offs and
settlements.

### BS-arap-1: Aging buckets: Current, 1-30, 31-60, 61-90, 90+ days past due (B-18)

- **Status:** CONVENTION (bucket edges); CHOICE (a document due today is Current; aging as of a day shows only documents issued by then, less payments made by then; a payment never predates its document)
- **Statement:** An aging as of a day MUST place each open document in exactly one of the buckets Current (not yet past due, including due today), 1-30, 31-60, 61-90 and 90+ days past due, and the buckets MUST partition what the documents owe, exactly. A void document is in no bucket; a payment reversed by the day no longer counts.
- **Sources:**
  - external: ASC 326-20-55-37 and 326-20-55-38, an illustrative aging of trade receivables
- **Symbolic form:** `d = as_of - due`; bucket `Current` if `d <= 0`, `1-30` if `1 <= d <= 30`, `31-60`, `61-90`, `90+` if `d >= 91`; `sum_b owed(b) = sum of open balances`.
- **Examples:** none yet. The schema has no aging query; the example for it waits on one (a document-level query is owed by this edition's successor).
- **Applicability:** accrual-basis entities with receivables or payables.

### BS-arap-2: A document's figures are its ledger lines that stand; a control account is its documents (B-38)

- **Status:** CHOICE (the owner, 2026-10-23); CONVENTION (a control account agrees with its subledger)
- **Statement:** Every ledger line on a receivables or payables control account MUST carry the document it belongs to; a document's total, paid and owed MUST be derived from its tagged lines that stand (a reversed line no longer counts); a control account's balance MUST equal the sum of its documents'. A hand line on a control account MUST be refused.
- **Sources:**
  - external: ASC 310-10 (receivables); ASC 405-20-40-1 (a liability is removed only when paid or released); ASC 606-10-45-4 (a receivable is a right to consideration that is unconditional)
- **Symbolic form:** `bal(1200) = sum_{doc} owed(doc)` and `owed(doc) = sum{signed tagged lines that stand}`.
- **Examples:** `arap-hand-control-posting-refused`
- **Applicability:** engines with invoices and bills.

### BS-arap-3: Terms: when a document is due (B-39)

- **Status:** reserved
- **Statement:** Reserved: allocated in the wanderbooks plan (8.0.3) for payment terms and due dates, not yet stated in `docs/spec/rules.md`.
- **Sources:**
  - none yet
- **Symbolic form:** none yet.
- **Examples:** none yet
- **Applicability:** none yet.

### BS-arap-4: Numbers and the issued copy (B-40)

- **Status:** reserved
- **Statement:** Reserved: allocated in the wanderbooks plan (8.0.3) for document numbers and the issued copy, not yet stated in `docs/spec/rules.md`.
- **Sources:**
  - none yet
- **Symbolic form:** none yet.
- **Examples:** none yet
- **Applicability:** none yet.

### BS-arap-5: A debt that will not be collected is written off, and a recovery is income (B-41)

- **Status:** CHOICE (direct write-off, the owner, 2026-10-23); options: direct write-off, allowance method
- **Statement:** On the direct method, an invoice that will not be collected MUST be written off by debiting bad debt expense and crediting receivables, the receivable line tagged with the invoice; a partial write-off is the amount charged off; a write-off MUST be refused on cash-basis books and for more than is owed; a later recovery is income.
- **Sources:**
  - external: 26 U.S.C. section 166(a); Treas. Reg. sections 1.166-1(e), 1.166-2(a), 1.166-3(a); 26 U.S.C. section 111 (recovery); ASC 326-20 (the allowance method, not applied)
- **Symbolic form:** `Dr 6990 amount; Cr 1200 amount` tagged with the invoice; `0 < amount <= owed`.
- **Examples:** `bad-debt-direct-write-off`
- **Applicability:** accrual-basis taxpayers.

### BS-arap-6: A settlement is exact, one party, and names its lines; the gross discount and the gross fee (B-42)

- **Status:** CONVENTION; CHOICE (the gross method for an early-payment discount, the owner A-8; the processor's fee posted gross, A-4)
- **Statement:** A settlement MUST apply to documents of one party and one kind, never the same document twice; sum of cash plus sum of discounts MUST equal what it applies to the documents, and the deposit MUST equal cash less fee, exactly; no application may exceed what its document owes on any day from the payment's date on; a discount MUST be at most the terms allow and be dated within the discount period, or carry a reason; a processor's fee posts gross (expense debited, deposit the net, receivables credited the gross).
- **Sources:**
  - external: ASC 606-10-32-5 to 32-9 (the strict treatment of an expected discount, simplified here by the gross method)
- **Symbolic form:** `sum cash + sum discount = sum applied`; `deposit = sum cash - fee`; `owed_after >= 0` on every day.
- **Examples:** none yet. The schema has no payment or settlement event (`arap-settlement-exact` and `processor-fee-gross-presentation` are the names the plan allocates; they wait on that event).
- **Applicability:** engines with documents and settlements.
