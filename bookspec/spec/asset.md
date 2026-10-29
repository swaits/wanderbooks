# Area: asset

Fixed assets, depreciation, the register and capitalization.

### BS-asset-1: Depreciation: straight line, salvage never depreciated, partial months by days (B-15)

- **Status:** CONVENTION (the book method); AUTHORITY for tax conventions, which an engine need not apply
- **Statement:** Depreciation MUST spread cost less salvage over the useful life in a systematic and rational manner: straight line, full months equal, the first and last partial months by their days, the last month taking what the others leave so the total is exactly cost less salvage. Salvage MUST NOT be depreciated. A run MUST post what the schedule says is due through the month and the books do not carry (a skipped month caught up); a disposal MUST post any depreciation still due through its day, then the disposal entry with its gain or loss; depreciation past a disposal MUST be refused.
- **Sources:**
  - vendored `sources/fasb-asc.md`: "360-10-35-4"
  - external: OpenStax, Principles of Accounting, Volume 1, section 11.3, straight-line depreciation
- **Symbolic form:** `dep_m = round((cost - salvage) * days_m / days_total)` for the partial months; `sum_m dep_m = cost - salvage`.
- **Examples:** `assets-acquired-before-in-service`, `assets-a-reversed-run-is-not-posted`, `assets-book-value-is-what-the-books-carry`, `depreciation-catch-up-month`, `depreciation-disposal-after-a-late-change-undone`, `depreciation-disposal-before-a-later-run`, `depreciation-disposal-in-final-month`, `depreciation-disposal-inside-a-posted-month`, `depreciation-disposal-mid-month`, `depreciation-final-month-by-days`, `depreciation-final-month-drift`, `depreciation-reversal-before-its-run-is-refused`, `depreciation-run-reversed-after-its-month`
- **Applicability:** entities with fixed assets; tax conventions (half-year, mid-month) are IRS Pub. 946's and are not applied.

### BS-asset-2: Expense or capitalize (book policy, mirrors tax) (B-22)

- **Status:** AUTHORITY
- **Statement:** An item costing no more than the de minimis safe harbor (500 dollars per invoice or item without an applicable financial statement; 2,500 dollars from taxable years beginning in 2016 under Notice 2015-82) MAY be expensed; a larger one SHOULD be capitalized and depreciated, and amounts paid for benefits lasting no more than twelve months MAY be expensed under the 12-month rule.
- **Sources:**
  - vendored `sources/irs-capitalization.md`: "1.263(a)-1(f)"
  - vendored `sources/irs-capitalization.md`: "1.263(a)-4(f)"
- **Symbolic form:** `cost <= safe_harbor implies expense MAY`; the election is made yearly on the return.
- **Examples:** none yet. This is a policy the bookkeeper applies; no engine enforces it, so no engine-neutral behaviour can be demonstrated.
- **Applicability:** US taxpayers; thresholds as of 2026.

### BS-asset-3: Asset tags: one label per asset, never reused (B-24)

- **Status:** CHOICE (the owner, 2026-10-15)
- **Statement:** An asset tag (1 to 32 ASCII letters, digits and `- _ . /`) MUST be unique ignoring case and MUST NOT be reused by another asset, even after a retag or a disposal. The fixed asset register MUST reconcile to the cost and accumulated depreciation accounts, showing unregistered cost, unposted depreciation and unattributed accumulated depreciation.
- **Sources:**
  - external: CHOICE only: the owner, 2026-10-15 (docs/spec/rules.md B-24); no authority governs a label
- **Symbolic form:** `tag(a) = tag(b) implies a = b` (case folded), through the asset's whole life.
- **Examples:** none yet. The schema registers assets without tags; the register-row and register-totals queries demonstrate the reconciliation under BS-asset-1.
- **Applicability:** engines with an asset register; an engine declares its pick.

### BS-asset-4: Tax figures beside book value: the section 723 basis, the depreciable basis, the section 704(c) built-in gain (B-25)

- **Status:** AUTHORITY; CHOICE (the register carries tax basis; D-41)
- **Statement:** For property a member contributes, the partnership's basis MUST be the member's adjusted basis (carryover, section 723); book value is its fair market value; the difference at contribution is the section 704(c) built-in gain, fixed at contribution; the depreciable basis of property converted from personal use is the lesser of fair market value and adjusted basis.
- **Sources:**
  - vendored `sources/irs-partnership-basis.md`: "723"
- **Symbolic form:** `basis_723 = adjusted_basis`; `built_in = fmv - basis_723` at contribution; `depreciable = min(fmv, adjusted_basis)` for personal-use conversion.
- **Examples:** `assets-723-basis-apart-from-depreciable-basis`, `assets-built-in-gain-fixed-at-contribution`, `assets-four-contributed-devices`
- **Applicability:** partnerships receiving contributed property.
