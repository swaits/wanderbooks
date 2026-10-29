# Area: period

Periods and closes, and what an omitted date means.

### BS-period-1: Revenue and expenses close into profit and loss, and that into capital (B-11)

- **Status:** AUTHORITY (the mechanics); the account the net lands in depends on the entity
- **Statement:** Closing a period MUST post an entry that brings every revenue and expense account's balance through the period's last day to zero and credits equity with exactly revenue less expenses; the entry MUST balance. A corporation closes into retained earnings, a partnership into each partner's capital (BS-partner-1).
- **Sources:**
  - vendored `sources/pacioli-1494.md`: "must then be transferred for its closing (saldo) into the capital account"
  - external: OpenStax, Principles of Accounting, Volume 1, section 5.1: temporary accounts are closed at the end of each accounting period
- **Symbolic form:** for nominal `n`: `bal_n(close) = 0`; `E' = E + (R - X)` (`Bookspec.law_net_income_articulates`).
- **Examples:** `period-a-close-sweeps-the-year-into-capital`
- **Applicability:** every entity; the target account differs by entity type.

### BS-period-2: A closed period takes no new postings; periods close in order (B-12)

- **Status:** CONVENTION
- **Statement:** An entry dated within a closed period MUST be refused. Periods MUST close in order, and MUST nest (a month in a quarter in a year) and never partially overlap.
- **Sources:**
  - vendored `sources/irs-periods-and-methods.md`: "A tax year is an annual accounting period for keeping records and reporting income and expenses"
- **Symbolic form:** `date(E) in closed(P) implies post(E)` is refused; `P, Q` periods: nested or disjoint.
- **Examples:** `period-a-close-sweeps-the-year-into-capital`
- **Applicability:** every engine that closes.

### BS-period-3: A report asked for without dates covers this fiscal year so far (B-30)

- **Status:** AUTHORITY (the period); CHOICE (the default, the owner, 2026-10-20)
- **Statement:** A report asked for with no dates MUST cover the fiscal year containing today, from its first day through today; an omitted start is the first day of the fiscal year holding the end; a start after today with no end MUST be refused; an end before the start MUST be refused.
- **Sources:**
  - vendored `sources/irs-periods-and-methods.md`: "A tax year is an annual accounting period for keeping records and reporting income and expenses"
- **Symbolic form:** `to = today`; `from = fiscal_year_start(to)`; `from <= to`.
- **Examples:** `default-period-fiscal-year-to-date`
- **Applicability:** every engine with a fiscal-year setting.
