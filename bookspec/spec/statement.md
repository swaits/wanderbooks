# Area: statement

Reports: the trial balance, the accounting equation, the period
statements, the package as of a day, the project.

### BS-statement-1: The trial balance balances (B-6)

- **Status:** AUTHORITY
- **Statement:** The trial balance MUST list each account's balance as a debit or a credit, and the sum of the debits MUST equal the sum of the credits, as of every day.
- **Sources:**
  - vendored `sources/pacioli-1494.md`: "should be equal to the total of the debits"
  - vendored `sources/irs-records.md`: "the total debits must equal the total credits"
- **Symbolic form:** `sum{max(0, D_a - C_a)} = sum{max(0, C_a - D_a)}`, equivalently `sum_a (D_a - C_a) = 0` (`Bookspec.law_trial_balance`).
- **Examples:** `statement-the-books-balance-and-the-equation-holds`
- **Applicability:** every engine.

### BS-statement-2: The accounting equation (B-7)

- **Status:** AUTHORITY
- **Statement:** At every date, assets MUST equal liabilities plus equity plus revenue less expenses, each on its normal side. Income not yet closed is part of equity.
- **Sources:**
  - external: FASB Concepts Statement No. 8, Chapter 4, paragraph E62: equity is the difference between the entity's assets and its liabilities
- **Symbolic form:** `A = L + E + (R - X)` (`Bookspec.law_accounting_equation`).
- **Examples:** `statement-the-books-balance-and-the-equation-holds`
- **Applicability:** every engine.

### BS-statement-3: Statements: P&L for a period; cash flows by the direct method (B-14)

- **Status:** AUTHORITY, except where marked CONVENTION or CHOICE
- **Statement:** The profit and loss for a period MUST report revenue and expense activity within it and MUST exclude closing entries. The statement of cash flows MUST report the cash receipts and payments of the period by operating, investing and financing activity (the direct method); transfers among cash accounts MUST NOT be flows; the three sections MUST sum to the change in the cash accounts' balances; noncash investing and financing MUST be disclosed apart.
- **Sources:**
  - external: ASC 230-10-45-1 (cash effects of operating, investing and financing); 230-10-45-4 and 45-5 (cash and cash equivalents; transfers among them are not cash flows); 230-10-45-14 (cash receipts and payments from operating activities); 230-10-45-25 (classes of operating cash payments); 230-10-50-3 (noncash investing and financing)
  - vendored `sources/fasb-asc.md`: "230-10-45"
- **Symbolic form:** `operating + investing + financing = cash_end - cash_begin` (`Bookspec.law_cash_flow_is_change_in_cash`); an entry contributes its net movement of the cash accounts only.
- **Examples:** `cash-flow.debt-to-equity`, `cash-flow.government-fees`, `cash-flow.paid-on-the-companys-behalf`, `cash-flow.part-cash-purchase`, `cash-flow.reversal-of-a-reversal`, `cash-flow.reversed-property-contribution`, `cash-flow.two-member-llc-first-year`, `period-a-close-sweeps-the-year-into-capital`
- **Applicability:** accrual and cash basis entities reporting under US GAAP.

### BS-statement-4: A package as of a day shows the books through that day (B-33)

- **Status:** CONVENTION; CHOICE for the 1099 worksheet (the owner, 2026-10-20)
- **Statement:** A report or package made as of a day MUST reflect nothing dated after it: no figure, entry, receipt or worksheet row. Balances run through the day; a period's activity from the period's start through it.
- **Sources:**
  - vendored `sources/irs-periods-and-methods.md`: "A tax year is an annual accounting period for keeping records and reporting income and expenses"
- **Symbolic form:** `report(as_of) = report(books restricted to date <= as_of)`.
- **Examples:** `as-of-package-stops-at-as-of`
- **Applicability:** every engine that produces a package.

### BS-statement-5: A project is a class with its facts; its figures are its tagged lines (B-37)

- **Status:** CHOICE (the owner, 2026-10-21); CONVENTION (job costing); AUTHORITY (costs of work on spec are expensed)
- **Statement:** A project's revenue, expenses and net MUST be exactly the revenue and expense lines tagged with it, closes left out; its to-date figures MUST be every tagged line through the date; an on-spec project's unrecovered cost MUST be its expenses less its revenue to date, never below zero.
- **Sources:**
  - external: ASC 340-40-25-5 and 340-40-25-7 (costs to fulfil a contract); ASC 450-30-25-1 (gain contingencies)
- **Symbolic form:** `net(p) = sum{signed credit-normal value of tagged revenue and expense lines of p}`.
- **Examples:** `projects-invoice-and-bill-lines-count-by-project`, `projects-net-is-its-tagged-lines`, `projects-on-spec-costs-are-expenses`
- **Applicability:** engines with classes or projects.
