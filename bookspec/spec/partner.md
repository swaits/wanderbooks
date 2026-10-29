# Area: partner

Members of a partnership or LLC taxed as one, and the parties the books
deal with.

### BS-partner-1: Partners share income by the partnership agreement; capital accounts carry it (B-13)

- **Status:** AUTHORITY (shares, capital accounts); CHOICE (cent rounding, 2026-10-13; counting from the first close, 2026-10-16; the statement splits unclosed earnings the same running way, 2026-10-20)
- **Statement:** A partner's distributive share of income MUST be determined by the partnership agreement. Each member's capital account MUST be one running balance of contributions, allocations and distributions; a close MUST allocate the period's net income among members by their shares, the shares summing to one. An allocation MUST sum to exactly the net, each member's share rounded to the currency's smallest unit by the largest-remainder rule and a running split carrying what earlier closes gave.
- **Sources:**
  - vendored `sources/irs-partnerships.md`: "A partner's distributive share of income, gain, loss, deduction, or credit shall, except as otherwise provided in this chapter, be determined by the partnership agreement."
  - vendored `sources/irs-capital-accounts.md`: "capital account"
- **Symbolic form:** `sum_k share_k = 1`; `part_k(t) = round_LR(share_k * N(t)) - sum_{s<t} part_k(s)` with `sum_k part_k(t) = N(t) - N(t-1)`.
- **Examples:** `members-close.odd-cent-across-closes`, `members-close.statement-before-and-after-a-close`, `members-close.thirds-over-twelve-months`, `members-close.true-up-of-a-per-close-split`, `p3-party-members-equity-and-k1`
- **Applicability:** entities taxed as partnerships; a count ends where the members or shares change.

### BS-partner-2: Parties: one directory, no full TIN in the books (B-26)

- **Status:** AUTHORITY (truncated TINs); CHOICE (the owner, 2026-10-15)
- **Statement:** Everyone the company deals with MUST be a party with roles. Names MUST be unique among live parties, ignoring capitals and surrounding space. A party record MUST hold at most the last four digits of a TIN; the books MUST NOT hold a nine-digit number in any party field, and a full TIN lives only in a vault outside the ledger.
- **Sources:**
  - vendored `sources/irs-truncated-tin.md`: "truncated"
  - vendored `sources/irs-tin-formats.md`: "taxpayer identification number"
- **Symbolic form:** `tin_last4(p) in [0-9]{4}`; no field of a party matches `[0-9]{9}`.
- **Examples:** `p3-party-tin-last4`
- **Applicability:** US persons with TINs.

### BS-partner-3: A party's part of an entry, and a party's totals (B-35)

- **Status:** CONVENTION (the sign); CHOICE (what counts as money, the owner, 2026-10-15 and 2026-10-20); AUTHORITY (what a member's capital counts; a promise is not payment)
- **Statement:** An entry's part for a party MUST be signed as a line on the party's own account would be (paid or given positive, received negative); money paid down on a liability counts once; an accrual counts once; a liability credit MUST NOT count as paying that liability's own creditor; a promise MUST NOT count as payment.
- **Sources:**
  - vendored `sources/helvering-v-price-1940.md`: "Helvering v. Price"
  - vendored `sources/irs-capital-accounts.md`: "capital account"
- **Symbolic form:** `part(E, p) = sum{signed lines of E on the accounts that are p's money}`; totals are sums over entries on each entry's first original's side.
- **Examples:** `p3-party-accrual-counts-once`, `p3-party-distribution-still-owed`, `p3-party-gift-card-is-a-promise`, `p3-party-members-equity-and-k1`, `p3-party-own-liability-never-pays`, `p3-party-reversed-card-payment`, `p3-party-signs-and-reversals`
- **Applicability:** every engine with parties.

### BS-partner-4: The K-1 worksheet reports tax-basis capital beside book capital (B-36)

- **Status:** AUTHORITY; CHOICE (D-41, the owner, 2026-10-20)
- **Statement:** The K-1 worksheet MUST report each member's capital on the tax basis (beginning, contributions at adjusted tax basis, share of income for tax purposes, other changes, distributions, ending) beside the book figures; a line the books cannot know MUST be shown open, never guessed.
- **Sources:**
  - vendored `sources/irs-partnerships.md`: "Tax-basis method"
  - vendored `sources/irs-partnership-basis.md`: "723"
- **Symbolic form:** `ending = beginning + contributions + income + other - distributions`, per member, on each basis.
- **Examples:** `partnership-k1-tax-basis-capital`, `partnership-k1-tax-basis-odd-cents`
- **Applicability:** partnerships filing Form 1065.
