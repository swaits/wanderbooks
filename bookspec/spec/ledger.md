# Area: ledger

The chart, the journal, posted-is-forever, corrections, control roles and
how the books are kept.

### BS-ledger-1: One money through the ledger; foreign amounts at the transaction-date rate (B-3)

- **Status:** AUTHORITY (the single money, the rate); CHOICE (booking in the smallest unit, the owner, 2026-10-13)
- **Statement:** A ledger MUST be kept in one base currency. A foreign amount MUST be booked at the rate in effect on the transaction date, rounded half away from zero to the base currency's smallest unit, and the original amount and rate kept. A line finer than the smallest unit MUST be refused, never rounded silently.
- **Sources:**
  - vendored `sources/pacioli-1494.md`: "the money that you reckon by at the beginning should be the same all through the Ledger"
  - vendored `sources/iso-4217.md`: "Minor units" (the table of currencies by minor-unit digits; two for the US dollar)
  - external: ASC 830-20-30-1, initial measurement in the functional currency at the rate in effect at the transaction date
- **Symbolic form:** `base = round_half_away(foreign * rate, u)`; every booked amount `in Z * u`.
- **Examples:** `ledger-a-fraction-of-a-cent-is-refused`
- **Applicability:** every engine. The single-currency refusal is demonstrated; the foreign-currency booking has no example in the first edition (the schema has no foreign line).

### BS-ledger-2: Five elements, closed list (B-4)

- **Status:** AUTHORITY (the definitions); CONVENTION (the five-type chart); CHOICE (a child has its parent's type, the owner, 2026-10-13)
- **Statement:** Every account MUST be exactly one of asset, liability, equity, revenue, expense. An account under a parent MUST have its parent's type.
- **Sources:**
  - external: FASB Concepts Statement No. 8, Chapter 4, paragraphs E16 (asset), E37 (liability), E61 (equity), E80 (revenue), E81 (expense)
- **Symbolic form:** `type(a) in {asset, liability, equity, revenue, expense}`; `parent(a) = p implies type(a) = type(p)`.
- **Examples:** `ledger-a-child-account-has-its-parents-type`
- **Applicability:** every engine; engines with a finer chart (contra accounts, memo accounts) map onto the five.

### BS-ledger-3: Normal balances (B-5)

- **Status:** CONVENTION
- **Statement:** An asset or expense account's balance MUST be shown as debits less credits, a liability, equity or revenue account's as credits less debits.
- **Sources:**
  - vendored `sources/pacioli-1494.md`: "This capital must always be placed as creditor"
  - external: OpenStax, Principles of Accounting, Volume 1, section 3.1
- **Symbolic form:** `bal(a) = D(a) - C(a)` if `type(a) in {asset, expense}`, else `C(a) - D(a)`.
- **Examples:** `statement-the-books-balance-and-the-equation-holds`
- **Applicability:** every engine.

### BS-ledger-4: Posted is forever: the book is written only forward (B-8)

- **Status:** AUTHORITY
- **Statement:** A posted entry MUST NOT be changed or deleted. The ledger at a later time MUST contain every earlier posted entry unchanged, and a change to a record MUST leave the audit trail intact.
- **Sources:**
  - vendored `sources/pacioli-1494.md`: "You should not write backward, but always forward"
  - vendored `sources/irs-records.md`: "A taxpayer establishes this reconciliation by demonstrating the relationship"
  - external: Handelsgesetzbuch (HGB) section 239(3), official English translation: an entry may not be modified such that its original content can no longer be established
- **Symbolic form:** for ledger states `S_t` and `t <= t'`: `posted(S_t)` is a prefix of `posted(S_t')`.
- **Examples:** `ledger-a-mistake-is-reversed-never-erased`
- **Applicability:** every engine.

### BS-ledger-5: A mistake is corrected by an opposite entry, never erased (B-9)

- **Status:** AUTHORITY
- **Statement:** A mistaken entry MUST be corrected by a reversal: a new entry with the same accounts and amounts and the sides flipped, linked to the original. The original MUST remain as it was written, and the pair MUST net to zero on every account.
- **Sources:**
  - vendored `sources/pacioli-1494.md`: "entry opposite this one in the credit for the same amount"
- **Symbolic form:** `reverse(E) = {(flip(s_i), a_i)}`; per account `signed(E) + signed(reverse(E)) = 0`.
- **Examples:** `ledger-a-mistake-is-reversed-never-erased`
- **Applicability:** every engine.

### BS-ledger-6: An entry is reversed at most once; a closing entry never (B-10)

- **Status:** CHOICE (the owner, 2026-10-13); options: reversible once, reversible repeatedly; a close never reversible
- **Statement:** An entry MUST be reversed at most once; a second reversal of it MUST be refused. A closing entry MUST NOT be reversed, because a closed period stays closed. To restore a reversed entry, post a new entry.
- **Sources:**
  - vendored `sources/pacioli-1494.md`: "entry opposite this one in the credit for the same amount"
- **Symbolic form:** `|{R : reversal_of(R) = E}| <= 1`; `closing(E) implies no R with reversal_of(R) = E`.
- **Examples:** `ledger-a-mistake-is-reversed-never-erased`
- **Applicability:** an engine declares its pick (`claims.choices`).

### BS-ledger-7: Every transaction has supporting documents; records are kept (B-20)

- **Status:** AUTHORITY
- **Statement:** Every entry MUST have supporting documents, kept for the period of limitations of the return it supports (three years generally; seven for a bad debt). Evidence the books file with an entry MUST stay with it; a reversal needs none of its own.
- **Sources:**
  - vendored `sources/irs-records.md`: "In addition, you must keep supporting documents"
- **Symbolic form:** `forall posted E, not reversal(E): |documents(E)| >= 1`.
- **Examples:** none yet. The example schema has no evidence event, and wanderbooks enforces this at its tool layer, not in the ledger core; a first-edition example would assert a policy of the tool surface, which is not engine-neutral.
- **Applicability:** a US taxpayer keeping records for a return.

### BS-ledger-8: Control roles: one account each, of the right kind, moved only when empty (B-23)

- **Status:** CONVENTION (a control account agrees with the records it controls); CHOICE (default codes, the owner, 2026-10-13 and 2026-10-15)
- **Statement:** An engine that posts by role (receivables, payables, revenue, and the like) MUST give each role exactly one account, postable and of the role's type, and MUST refuse to move a role to another account while the old one holds a balance. A control account's lines MUST each belong to the document that put them there; a hand line on one MUST be refused.
- **Sources:**
  - external: ASC 310-10 (receivables) and ASC 405-20-40-1 (a liability is removed only when paid or released)
- **Symbolic form:** `role` is an injective partial map from roles to accounts; `bal(control(r)) = sum of open documents of r`.
- **Examples:** `arap-hand-control-posting-refused`
- **Applicability:** engines with documents (invoices, bills).

### BS-ledger-9: The books keep how they are kept (B-29)

- **Status:** CHOICE (the owner, 2026-10-16)
- **Statement:** The settings, conventions, decisions and open items that govern the books MUST be facts in the books, each with who decided it and when, so that replaying the books gives the same results without any state held elsewhere.
- **Sources:**
  - external: IRS Publication 538, Accounting Periods and Methods, "Accounting Method": the method is a matter of the books
- **Symbolic form:** `state = fold(apply, empty, journal)`.
- **Examples:** none yet. The schema's setup is the example's own, not a journal; replay equivalence is checked by the wanderbooks adapter (`Store::verify`) after every example, not stated as an engine-neutral expectation.
- **Applicability:** engines that keep a journal.

### BS-ledger-10: Renaming an account changes its label, nothing else (B-31)

- **Status:** CHOICE (the owner, 2026-10-20)
- **Statement:** An account MUST be renamed without changing any entry, line, balance or statement amount; every report, past periods included, prints the new name. A blank name, an unchanged name, a sibling's name (ignoring capitals and surrounding space), a name holding a nine-digit number and a name that is not one line MUST be refused.
- **Sources:**
  - vendored `sources/irs-records.md`: "A taxpayer establishes this reconciliation by demonstrating the relationship"
  - vendored `sources/intuit-iif.md`: "Accounts are identified by NAME"
- **Symbolic form:** `rename(a, n)` leaves `lines`, `bal` unchanged; `name(a) = n`.
- **Examples:** `rename-changes-no-amount`
- **Applicability:** every engine.

### BS-ledger-11: Every revenue and expense account maps to a line of the return (B-32)

- **Status:** AUTHORITY (why it matters); CHOICE (warn versus refuse, the owner, 2026-10-20); options: warn, refuse
- **Statement:** A revenue or expense account (not a header) SHOULD carry a tax category linking it to a line of the return; under the refuse policy an engine MUST refuse to create one without a category or to clear one. A header needs none.
- **Sources:**
  - vendored `sources/irs-records.md`: "Your books must show your gross income, as well as your deductions and credits."
- **Symbolic form:** `policy = refuse implies forall a in nominal, not header(a): category(a) defined`.
- **Examples:** `tax-category-required`
- **Applicability:** a US business filing a return.

### BS-ledger-12: Evidence: extraction, dedup, annotations, settlements (B-44)

- **Status:** CONVENTION
- **Statement:** Text and facts read off a receipt MUST be derived records the books can re-make, the file's bytes staying the authority; identical bytes MUST be one file wherever they are evidence; an annotation MUST change no amount; a payment's settlements MUST be named, never guessed.
- **Sources:**
  - external: Revenue Procedure 98-25, section 5.01, machine-sensible records with an audit trail
  - vendored `sources/irs-records.md`: "In addition, you must keep supporting documents"
- **Symbolic form:** `text(file) = f(bytes(file), extractor, version)`; `bytes(a) = bytes(b) implies file(a) = file(b)`.
- **Examples:** none yet. The example schema has no attachment or file event; an evidence example waits on a schema extension (the unit's step 5 names it as owed).
- **Applicability:** engines that hold receipts.

### BS-ledger-13: Audits and attestations (B-45)

- **Status:** CONVENTION
- **Statement:** An audit of the books through a day MUST give the same findings for the same books and programme; a claim that someone checked MUST rest on the books as they were when the audit ran, MUST NOT count when what it read disagrees with the books, and its method is the claimant's own word.
- **Sources:**
  - external: Revenue Procedure 98-25, section 5.01, machine-sensible records with an audit trail
  - vendored `sources/irs-records.md`: "In addition, you must keep supporting documents"
- **Symbolic form:** `findings(books, through) = f(books, through, programme)`; `counted(a, bar) = meets(method(a), bar) and not disagrees(a)`.
- **Examples:** none yet. The example schema has no audit event or query; an audit example waits on a schema extension.
- **Applicability:** engines that are audited.
