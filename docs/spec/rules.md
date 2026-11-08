# Rule catalog

Every rule the engine enforces, the authority for it, and what checks it.
Status is **AUTHORITY** (a source states the rule), **CONVENTION** (common
practice no authority states; the best reference is given), or
**CHOICE** (a human chose, and when; bookspec's CHOICE: the specification
names the options, an engine declares its pick). Vendored sources are in
`sources/`; FASB and OpenStax are quoted briefly under their copyright and
license (see `README.md`). Quotes from FASB's Codification were read in
Deloitte's DART roadmaps, which reproduce it ("FASB Accounting Standards
Codification: Copyright © 2005 - 2026 by Financial Accounting
Foundation").

Spec blocks in the code cite these as `SOURCE: rules.md B-n`. Every rule
also has a bookspec identifier, `**bookspec:** BS-<area>-<n>`: the table
from B-1 to B-45 is `spec/rules-index.md` in the bookspec repository, https://git.swaits.com/swaits/bookspec (P13, D-34).

---

## Index

Every rule's claims live in `claims/`, one file per bookspec area (P14 unit S-D0). This file is the index until unit S-E rewrites the `rules.md B-n` references to bookspec ids; the rule text itself is in the claims files.

| B | bookspec id | area | file |
|---|---|---|---|
| B-1 | `BS-entry-1` | entry | [claims/entry.md](claims/entry.md#b-1-every-entry-is-a-double-entry-debits-equal-credits) |
| B-2 | `BS-entry-2` | entry | [claims/entry.md](claims/entry.md#b-2-a-lines-amount-is-positive-its-side-gives-the-direction) |
| B-3 | `BS-ledger-1` | ledger | [claims/ledger.md](claims/ledger.md#b-3-one-money-through-the-ledger-foreign-amounts-at-the-transaction-date-rate) |
| B-4 | `BS-ledger-2` | ledger | [claims/ledger.md](claims/ledger.md#b-4-five-elements-closed-list) |
| B-5 | `BS-ledger-3` | ledger | [claims/ledger.md](claims/ledger.md#b-5-normal-balances) |
| B-6 | `BS-statement-1` | statement | [claims/statement.md](claims/statement.md#b-6-the-trial-balance-balances) |
| B-7 | `BS-statement-2` | statement | [claims/statement.md](claims/statement.md#b-7-the-accounting-equation) |
| B-8 | `BS-ledger-4` | ledger | [claims/ledger.md](claims/ledger.md#b-8-posted-is-forever-the-book-is-written-only-forward) |
| B-9 | `BS-ledger-5` | ledger | [claims/ledger.md](claims/ledger.md#b-9-a-mistake-is-corrected-by-an-opposite-entry-never-erased) |
| B-10 | `BS-ledger-6` | ledger | [claims/ledger.md](claims/ledger.md#b-10-an-entry-is-reversed-at-most-once-a-closing-entry-never) |
| B-11 | `BS-period-1` | period | [claims/period.md](claims/period.md#b-11-revenue-and-expenses-close-into-profit-and-loss-and-that-into-capital) |
| B-12 | `BS-period-2` | period | [claims/period.md](claims/period.md#b-12-a-closed-period-takes-no-new-postings-periods-close-in-order) |
| B-13 | `BS-partner-1` | partner | [claims/partner.md](claims/partner.md#b-13-partners-share-income-by-the-partnership-agreement-capital-accounts-carry-it) |
| B-14 | `BS-statement-3` | statement | [claims/statement.md](claims/statement.md#b-14-statements-pl-for-a-period-cash-flows-by-the-direct-method) |
| B-15 | `BS-asset-1` | asset | [claims/asset.md](claims/asset.md#b-15-depreciation-straight-line-salvage-never-depreciated-partial-months-by-days) |
| B-16 | `BS-prepaid-1` | prepaid | [claims/prepaid.md](claims/prepaid.md#b-16-prepaid-expenses-and-deferred-revenue-are-recognized-over-the-period-benefited-or-earned) |
| B-17 | `BS-revenue-1` | revenue | [claims/revenue.md](claims/revenue.md#b-17-platform-payouts-are-recorded-gross-when-the-business-is-the-principal) |
| B-18 | `BS-arap-1` | arap | [claims/arap.md](claims/arap.md#b-18-aging-buckets-current-130-3160-6190-90-days-past-due) |
| B-19 | `BS-1099-1` | 1099 | [claims/1099.md](claims/1099.md#b-19-1099-reporting-thresholds-payment-methods-and-decisions) |
| B-20 | `BS-ledger-7` | ledger | [claims/ledger.md](claims/ledger.md#b-20-every-transaction-has-supporting-documents-records-are-kept) |
| B-21 | `BS-entry-3` | entry | [claims/entry.md](claims/entry.md#b-21-exact-amounts-in-the-books-rounding-only-on-returns) |
| B-22 | `BS-asset-2` | asset | [claims/asset.md](claims/asset.md#b-22-expense-or-capitalize-book-policy-mirrors-tax) |
| B-23 | `BS-ledger-8` | ledger | [claims/ledger.md](claims/ledger.md#b-23-control-roles-one-account-each-of-the-right-kind-moved-only-when-empty) |
| B-24 | `BS-asset-3` | asset | [claims/asset.md](claims/asset.md#b-24-asset-tags-one-label-per-asset-never-reused) |
| B-25 | `BS-asset-4` | asset | [claims/asset.md](claims/asset.md#b-25-tax-figures-beside-book-value-the-723-basis-the-depreciable-basis-the-704c-built-in-gain) |
| B-26 | `BS-partner-2` | partner | [claims/partner.md](claims/partner.md#b-26-parties-one-directory-no-full-tin-in-the-books) |
| B-27 | `BS-format-1` | format | [claims/format.md](claims/format.md#b-27-the-journal-format-gate-newer-is-not-corrupt) |
| B-28 | `BS-entry-4` | entry | [claims/entry.md](claims/entry.md#b-28-what-new-commands-take-four-digit-years-amounts-a-line-can-carry) |
| B-29 | `BS-ledger-9` | ledger | [claims/ledger.md](claims/ledger.md#b-29-the-books-keep-how-they-are-kept) |
| B-30 | `BS-period-3` | period | [claims/period.md](claims/period.md#b-30-a-report-asked-for-without-dates-covers-this-fiscal-year-so-far) |
| B-31 | `BS-ledger-10` | ledger | [claims/ledger.md](claims/ledger.md#b-31-renaming-an-account-changes-its-label-nothing-else) |
| B-32 | `BS-ledger-11` | ledger | [claims/ledger.md](claims/ledger.md#b-32-every-revenue-and-expense-account-maps-to-a-line-of-the-return) |
| B-33 | `BS-statement-4` | statement | [claims/statement.md](claims/statement.md#b-33-a-package-as-of-a-day-shows-the-books-through-that-day) |
| B-34 | `BS-format-2` | format | [claims/format.md](claims/format.md#b-34-the-quickbooks-desktop-file-iif-plain-ascii-every-entry-through-as_of-closes-left-out-classes-carried) |
| B-35 | `BS-partner-3` | partner | [claims/partner.md](claims/partner.md#b-35-a-partys-part-of-an-entry-and-a-partys-totals) |
| B-36 | `BS-partner-4` | partner | [claims/partner.md](claims/partner.md#b-36-the-k-1-worksheet-reports-tax-basis-capital-beside-book-capital) |
| B-37 | `BS-statement-5` | statement | [claims/statement.md](claims/statement.md#b-37-a-project-is-a-class-with-its-facts-its-figures-are-its-tagged-lines) |
| B-38 | `BS-arap-2` | arap | [claims/arap.md](claims/arap.md#b-38-a-documents-figures-are-its-ledger-lines-that-stand-a-control-account-is-its-documents) |
| B-39 | `BS-arap-3` | arap | [claims/arap.md](claims/arap.md#b-39-terms-give-one-due-date-and-one-discount-date-exactly) |
| B-40 | `BS-arap-4` | arap | [claims/arap.md](claims/arap.md#b-40-an-invoice-number-names-one-invoice-forever-the-copy-sent-is-kept-a-vendor-invoice-is-entered-once) |
| B-41 | `BS-arap-5` | arap | [claims/arap.md](claims/arap.md#b-41-a-debt-that-will-not-be-collected-is-written-off-and-a-recovery-is-income) |
| B-42 | `BS-arap-6` | arap | [claims/arap.md](claims/arap.md#b-42-a-settlement-is-exact-and-names-its-lines) |
| B-43 | `BS-entry-5` | entry | [claims/entry.md](claims/entry.md#b-43-a-draft-waits-on-its-evidence-and-one-that-will-never-post-can-be-discarded) |
| B-44 | `BS-ledger-12` | ledger | [claims/ledger.md](claims/ledger.md#b-44-evidence-is-read-from-the-bytes-never-trusted-over-them-identical-files-are-kept-once) |
| B-45 | `BS-ledger-13` | ledger | [claims/ledger.md](claims/ledger.md#b-45-an-audit-is-deterministic-over-the-books-an-attestation-is-a-claim-by-a-named-client) |
