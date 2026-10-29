# Area: prepaid

Prepaid expenses and deferred revenue.

### BS-prepaid-1: Prepaid expenses and deferred revenue are recognized over the period benefited or earned (B-16)

- **Status:** AUTHORITY (revenue); CONVENTION (prepaids)
- **Statement:** A payment received before a good or service is transferred MUST be a contract liability, recognized as revenue as the performance obligation is satisfied; a prepaid expense MUST be an asset recognized as the benefit is used. A schedule MUST recognize exactly its total over its months: by the day (first and last partial months by their days, the last taking the remainder) or in equal months from the first.
- **Sources:**
  - external: ASC 606-10-45-2 (contract liability); ASC 606-10-25-23 (recognize revenue when a performance obligation is satisfied); ASC 340-10-05-4 and ASC 210-10-45-1(g)
  - vendored `sources/irs-capitalization.md`: "1.263(a)-4(f)"
- **Symbolic form:** `sum_m recognized_m = total`; `recognized_m >= 0`.
- **Examples:** `schedule-daily-deferred-revenue`, `schedule-daily-small-first-and-last-month`, `schedule-equal-months-start-on-the-first`
- **Applicability:** accrual-basis entities.
