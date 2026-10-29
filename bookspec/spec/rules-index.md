# bookspec's rule index

Every rule of `docs/spec/rules.md` (B-1 to B-45), its bookspec
identifier `BS-<area>-<n>` (`plan/served.md` P13 step 3), its area,
what its statement rests on and the examples that demonstrate it.

A rule whose only source is a human choice carries rules.md's **CHOICE**, bookspec's status of the same name:
the specification names the options, an engine declares its pick
(`claims.choices`, `bookspec_checker::Claims`). Rules B-39, B-40 and B-43
to B-45 are allocated in `plan/served.md` 8.0.3 and not yet stated: their
identifier and area are fixed here so a later unit fills them in
without moving anyone else's number.

| B | bookspec id | area | status | rule | examples |
|---|---|---|---|---|---|
| B-1 | `BS-entry-1` | entry | AUTHORITY | Every entry is a double entry: debits equal credits | — |
| B-2 | `BS-entry-2` | entry | CONVENTION | A line's amount is positive; its side gives the direction | — |
| B-3 | `BS-ledger-1` | ledger | AUTHORITY | One money through the ledger; foreign amounts at the transaction-date rate | — |
| B-4 | `BS-ledger-2` | ledger | AUTHORITY | Five elements, closed list | — |
| B-5 | `BS-ledger-3` | ledger | CONVENTION | Normal balances | — |
| B-6 | `BS-statement-1` | statement | AUTHORITY | The trial balance balances | — |
| B-7 | `BS-statement-2` | statement | AUTHORITY | The accounting equation | — |
| B-8 | `BS-ledger-4` | ledger | AUTHORITY | Posted is forever: the book is written only forward | — |
| B-9 | `BS-ledger-5` | ledger | AUTHORITY | A mistake is corrected by an opposite entry, never erased | — |
| B-10 | `BS-ledger-6` | ledger | CHOICE | An entry is reversed at most once; a closing entry never | — |
| B-11 | `BS-period-1` | period | AUTHORITY | Revenue and expenses close into profit and loss, and that into capital | — |
| B-12 | `BS-period-2` | period | CONVENTION | A closed period takes no new postings; periods close in order | — |
| B-13 | `BS-partner-1` | partner | AUTHORITY | Partners share income by the partnership agreement; capital accounts carry it | `members-close.odd-cent-across-closes`, `members-close.statement-before-and-after-a-close`, `members-close.thirds-over-twelve-months`, `members-close.true-up-of-a-per-close-split`, `p3-party-members-equity-and-k1` |
| B-14 | `BS-statement-3` | statement | AUTHORITY | Statements: P&L for a period; cash flows by the direct method | `cash-flow.debt-to-equity`, `cash-flow.government-fees`, `cash-flow.paid-on-the-companys-behalf`, `cash-flow.part-cash-purchase`, `cash-flow.reversal-of-a-reversal`, `cash-flow.reversed-property-contribution`, `cash-flow.two-member-llc-first-year` |
| B-15 | `BS-asset-1` | asset | CONVENTION | Depreciation: straight line, salvage never depreciated, partial months by days | `assets-acquired-before-in-service`, `assets-a-reversed-run-is-not-posted`, `assets-book-value-is-what-the-books-carry`, `depreciation-catch-up-month`, `depreciation-disposal-after-a-late-change-undone`, `depreciation-disposal-before-a-later-run`, `depreciation-disposal-in-final-month`, `depreciation-disposal-inside-a-posted-month`, `depreciation-disposal-mid-month`, `depreciation-final-month-by-days`, `depreciation-final-month-drift`, `depreciation-reversal-before-its-run-is-refused`, `depreciation-run-reversed-after-its-month` |
| B-16 | `BS-prepaid-1` | prepaid | AUTHORITY | Prepaid expenses and deferred revenue are recognized over the period benefited or earned | `schedule-daily-deferred-revenue`, `schedule-daily-small-first-and-last-month`, `schedule-equal-months-start-on-the-first` |
| B-17 | `BS-revenue-1` | revenue | AUTHORITY | Platform payouts are recorded gross when the business is the principal | — |
| B-18 | `BS-arap-1` | arap | CONVENTION | Aging buckets: Current, 1-30, 31-60, 61-90, 90+ days past due | — |
| B-19 | `BS-1099-1` | 1099 | AUTHORITY | 1099 reporting thresholds, payment methods, and decisions | `1099-a-dated-status-applies-by-payment-date`, `1099-a-members-check-through-financing-counts`, `1099-and-party-totals-for-a-members-payment`, `1099-card-payments-are-excluded`, `1099-money-between-own-accounts-pays-no-one`, `1099-money-from-a-payee-is-netted-only-as-a-refund`, `1099-the-threshold-is-the-years`, `1099-worksheet-through-as-of` |
| B-20 | `BS-ledger-7` | ledger | AUTHORITY | Every transaction has supporting documents; records are kept | — |
| B-21 | `BS-entry-3` | entry | AUTHORITY | Exact amounts in the books; rounding only on returns | — |
| B-22 | `BS-asset-2` | asset | AUTHORITY | Expense or capitalize (book policy, mirrors tax) | — |
| B-23 | `BS-ledger-8` | ledger | CONVENTION | Control roles: one account each, of the right kind, moved only when empty | — |
| B-24 | `BS-asset-3` | asset | CHOICE | Asset tags: one label per asset, never reused | — |
| B-25 | `BS-asset-4` | asset | AUTHORITY | Tax figures beside book value: the section 723 basis, the depreciable basis, the section 704(c) built-in gain | `assets-723-basis-apart-from-depreciable-basis`, `assets-built-in-gain-fixed-at-contribution`, `assets-four-contributed-devices` |
| B-26 | `BS-partner-2` | partner | AUTHORITY | Parties: one directory, no full TIN in the books | `p3-party-tin-last4` |
| B-27 | `BS-format-1` | format | CHOICE | The journal format gate: newer is not corrupt | — |
| B-28 | `BS-entry-4` | entry | CONVENTION | What new commands take: four-digit years, amounts a line can carry | — |
| B-29 | `BS-ledger-9` | ledger | CHOICE | The books keep how they are kept | — |
| B-30 | `BS-period-3` | period | AUTHORITY | A report asked for without dates covers this fiscal year so far | `default-period-fiscal-year-to-date` |
| B-31 | `BS-ledger-10` | ledger | CHOICE | Renaming an account changes its label, nothing else | `rename-changes-no-amount` |
| B-32 | `BS-ledger-11` | ledger | AUTHORITY | Every revenue and expense account maps to a line of the return | `tax-category-required` |
| B-33 | `BS-statement-4` | statement | CONVENTION | A package as of a day shows the books through that day | `as-of-package-stops-at-as-of` |
| B-34 | `BS-format-2` | format | AUTHORITY | The QuickBooks Desktop file (IIF): plain ASCII, every entry through as_of, closes left out, classes carried | `iif-quickbooks-journal-file` |
| B-35 | `BS-partner-3` | partner | CONVENTION | A party's part of an entry, and a party's totals | `p3-party-accrual-counts-once`, `p3-party-distribution-still-owed`, `p3-party-gift-card-is-a-promise`, `p3-party-members-equity-and-k1`, `p3-party-own-liability-never-pays`, `p3-party-reversed-card-payment`, `p3-party-signs-and-reversals` |
| B-36 | `BS-partner-4` | partner | AUTHORITY | The K-1 worksheet reports tax-basis capital beside book capital | `partnership-k1-tax-basis-capital`, `partnership-k1-tax-basis-odd-cents` |
| B-37 | `BS-statement-5` | statement | CHOICE | A project is a class with its facts; its figures are its tagged lines | `projects-invoice-and-bill-lines-count-by-project`, `projects-net-is-its-tagged-lines`, `projects-on-spec-costs-are-expenses` |
| B-38 | `BS-arap-2` | arap | CHOICE | A document's figures are its ledger lines that stand; a control account is its documents | `arap-hand-control-posting-refused` |
| B-39 | `BS-arap-3` | arap | — | Terms: when a document is due (allocated, not yet stated) | — |
| B-40 | `BS-arap-4` | arap | — | Numbers and the issued copy (allocated, not yet stated) | — |
| B-41 | `BS-arap-5` | arap | CHOICE | A debt that will not be collected is written off, and a recovery is income | `bad-debt-direct-write-off` |
| B-42 | `BS-arap-6` | arap | CONVENTION | A settlement is exact, one party, and names its lines; the gross discount and the gross fee | — |
| B-43 | `BS-entry-5` | entry | — | Drafts and evidence (allocated, not yet stated) | — |
| B-44 | `BS-ledger-12` | ledger | — | Evidence: extraction, dedup, annotations, settlements (stated in rules.md; its bookspec example comes with P13 step 5) | — |
| B-45 | `BS-ledger-13` | ledger | — | Audits and attestations (allocated, not yet stated) | — |
