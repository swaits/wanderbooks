# Golden worksheet — general ledger by account

Report: `general_ledger(&ledger, 2026-01-01, 2026-01-31)`.
Golden: `crates/ledger-core/tests/golden/general_ledger.json`.
Test: `golden_general_ledger` in
`crates/ledger-core/tests/golden_reports.rs`.

## Fixture

The shared extended copy of `fixture_company()`; see
`profit-and-loss-period.md` for the full table and why it was extended.
Entries 1–5 are the original fixture, 6 is a multi-leg purchase, 7 is the
January close, 8 is a February entry after the period end:

| # | Date | Description | Lines |
|---|------|-------------|-------|
| 1 | 2026-01-02 | Owner investment | Dr 1000 Cash 10,000.00; Cr 3000 Owner's Equity 10,000.00 |
| 2 | 2026-01-05 | Buy equipment | Dr 1500 Equipment 2,500.00; Cr 1000 Cash 2,500.00 |
| 3 | 2026-01-10 | Invoice client | Dr 1200 Accounts Receivable 4,000.00; Cr 4000 Service Revenue 4,000.00 |
| 4 | 2026-01-15 | January rent | Dr 6000 Rent Expense 800.00; Cr 1000 Cash 800.00 |
| 5 | 2026-01-20 | Client payment | Dr 1000 Cash 1,500.00; Cr 1200 Accounts Receivable 1,500.00 |
| 6 | 2026-01-25 | Buy more equipment | Dr 1500 Equipment 1,500.00; Cr 1000 Cash 1,000.00; Cr 2000 Accounts Payable 500.00 |
| 7 | 2026-01-31 | Close period 2026-01 (generated) | Dr 4000 Service Revenue 4,000.00; Cr 6000 Rent Expense 800.00; Cr 3999 Retained Earnings 3,200.00 |
| 8 | 2026-02-05 | February service | Dr 1200 Accounts Receivable 600.00; Cr 4000 Service Revenue 600.00 |

Accounts: 1000 Cash (Asset), 1200 Accounts Receivable (Asset), 1500
Equipment (Asset), 2000 Accounts Payable (Liability), 3000 Owner's Equity
(Equity), 4000 Service Revenue (Revenue), 6000 Rent Expense (Expense), 3999
Retained Earnings (Equity, created by the close). Entry ids are assigned in
posting order, 1–8; the close creates account 3999 without consuming an
entry id, and entry 8 is the February entry.

## Hand computation

The period is 2026-01-01 through 2026-01-31, both inclusive. Nothing is
dated before the period, so every beginning balance is 0. Entries 1–7 are
in the period and **entry 8 is after it and excluded**. The closing entry
7 is a posting like any other (the general ledger is not a P&L).

Splits (QuickBooks convention): the one account on the entry's opposite
side, else `-Split-` when that side has several accounts. Running balances
are on each account's normal side (assets and expenses debit, the rest
credit). Accounts are listed in code order.

### 1000 Cash — Asset, beginning 0

| Date | Entry | Description | Split | Debit | Credit | Balance |
|---|---|---|---|---|---|---|
| 2026-01-02 | 1 | Owner investment | 3000 Owner's Equity | 10,000.00 | 0 | 10,000.00 |
| 2026-01-05 | 2 | Buy equipment | 1500 Equipment | 0 | 2,500.00 | 7,500.00 |
| 2026-01-15 | 4 | January rent | 6000 Rent Expense | 0 | 800.00 | 6,700.00 |
| 2026-01-20 | 5 | Client payment | 1200 Accounts Receivable | 1,500.00 | 0 | 8,200.00 |
| 2026-01-25 | 6 | Buy more equipment | 1500 Equipment | 0 | 1,000.00 | 7,200.00 |

Debits 10,000 + 1,500 = 11,500; credits 2,500 + 800 + 1,000 = 4,300;
ending 11,500 − 4,300 = **7,200.00**.

### 1200 Accounts Receivable — Asset, beginning 0

| Date | Entry | Description | Split | Debit | Credit | Balance |
|---|---|---|---|---|---|---|
| 2026-01-10 | 3 | Invoice client | 4000 Service Revenue | 4,000.00 | 0 | 4,000.00 |
| 2026-01-20 | 5 | Client payment | 1000 Cash | 0 | 1,500.00 | 2,500.00 |

Debits 4,000; credits 1,500; ending **2,500.00**. (Entry 8's 600 debit is
dated 2026-02-05 and is excluded; including it would add a third posting
and an ending of 3,100.)

### 1500 Equipment — Asset, beginning 0

| Date | Entry | Description | Split | Debit | Credit | Balance |
|---|---|---|---|---|---|---|
| 2026-01-05 | 2 | Buy equipment | 1000 Cash | 2,500.00 | 0 | 2,500.00 |
| 2026-01-25 | 6 | Buy more equipment | -Split- | 1,500.00 | 0 | 4,000.00 |

Entry 6's Debit has **two** accounts on the opposite (credit) side, 1000
Cash and 2000 Accounts Payable, so its split is `-Split-`. Debits 4,000;
credits 0; ending **4,000.00**.

### 2000 Accounts Payable — Liability, beginning 0

| Date | Entry | Description | Split | Debit | Credit | Balance |
|---|---|---|---|---|---|---|
| 2026-01-25 | 6 | Buy more equipment | 1500 Equipment | 0 | 500.00 | 500.00 |

Debits 0; credits 500; ending **500.00**.

### 3000 Owner's Equity — Equity, beginning 0

| Date | Entry | Description | Split | Debit | Credit | Balance |
|---|---|---|---|---|---|---|
| 2026-01-02 | 1 | Owner investment | 1000 Cash | 0 | 10,000.00 | 10,000.00 |

Debits 0; credits 10,000; ending **10,000.00**.

### 3999 Retained Earnings — Equity, beginning 0

| Date | Entry | Description | Split | Debit | Credit | Balance |
|---|---|---|---|---|---|---|
| 2026-01-31 | 7 | Close period 2026-01 | 4000 Service Revenue | 0 | 3,200.00 | 3,200.00 |

The close's Credit has only 4000 Service Revenue on the opposite (debit)
side, so its split is that one account. Debits 0; credits 3,200; ending
**3,200.00**.

### 4000 Service Revenue — Revenue, beginning 0

| Date | Entry | Description | Split | Debit | Credit | Balance |
|---|---|---|---|---|---|---|
| 2026-01-10 | 3 | Invoice client | 1200 Accounts Receivable | 0 | 4,000.00 | 4,000.00 |
| 2026-01-31 | 7 | Close period 2026-01 | -Split- | 4,000.00 | 0 | 0.00 |

The close's Debit has two accounts on the opposite (credit) side, 6000 Rent
Expense and 3999 Retained Earnings, so its split is `-Split-`. Debits
4,000; credits 4,000; ending **0.00** (the account is still listed because
it has postings in the period).

### 6000 Rent Expense — Expense, beginning 0

| Date | Entry | Description | Split | Debit | Credit | Balance |
|---|---|---|---|---|---|---|
| 2026-01-15 | 4 | January rent | 1000 Cash | 800.00 | 0 | 800.00 |
| 2026-01-31 | 7 | Close period 2026-01 | 4000 Service Revenue | 0 | 800.00 | 0.00 |

Debits 800; credits 800; ending **0.00** (listed for its postings).

## Cross-check

Each ending balance equals the 2026-01-31 trial balance in
`trial-balance-as-of.md`: cash 7,200, receivables 2,500, equipment 4,000,
payable 500, owner's equity 10,000, retained earnings 3,200, revenue 0,
rent 0. The running balances are consistent at every step, and entry 8
(2026-02-05) never appears.
