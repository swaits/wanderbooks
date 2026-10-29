# Area: entry

The mechanics of one entry: its lines, their sides and amounts.

### BS-entry-1: Every entry is a double entry: debits equal credits (B-1)

- **Status:** AUTHORITY
- **Statement:** An entry MUST have at least two lines, and the sum of its debit lines MUST equal the sum of its credit lines exactly, with no tolerance. An entry that does not MUST be refused whole: it posts nothing, and leaves no partial entry behind.
- **Sources:**
  - vendored `sources/pacioli-1494.md`: "All entries made in the ledger have to be double entries"
  - vendored `sources/irs-records.md`: "the total debits must equal the total credits"
- **Symbolic form:** for an entry with lines `(s_i, a_i)`, `a_i > 0`: `n >= 2` and `sum{a_i : s_i = debit} = sum{a_i : s_i = credit}`.
- **Examples:** `entry-an-unbalanced-entry-is-refused`, `statement-the-books-balance-and-the-equation-holds`
- **Applicability:** every entity, basis and jurisdiction.

### BS-entry-2: A line's amount is positive; its side gives the direction (B-2)

- **Status:** CONVENTION
- **Statement:** A line's amount MUST be greater than zero, and its side, never its sign, MUST give the direction money moves. A line with a negative or zero amount MUST be refused.
- **Sources:**
  - vendored `sources/pacioli-1494.md`: "All the creditors must appear in the Ledger at the right hand side, and all the debtors at the left."
- **Symbolic form:** `a_i > 0`; `signed(i) = +a_i` if `s_i = debit`, `-a_i` if `s_i = credit`.
- **Examples:** `entry-a-line-amount-is-positive`
- **Applicability:** every engine. No authority forbids a negative amount outright; one way to write each line is chosen so every line reads one way.

### BS-entry-3: Exact amounts in the books; rounding only on returns (B-21)

- **Status:** AUTHORITY
- **Statement:** The books MUST keep every amount exactly, and a sum of amounts MUST be their exact sum. Rounding MUST NOT occur inside the books; where a rule needs a rounded figure (an allocation, a return line), it names the rounding mode and rounds only the total.
- **Sources:**
  - vendored `sources/irs-partnerships.md`: "include cents when adding the amounts and round off only the total"
- **Symbolic form:** amounts are elements of `Z * u` (`u` the currency's smallest unit); `+` is exact on them.
- **Examples:** `entry-cents-add-exactly`
- **Applicability:** every engine. Binary floating point is not an implementation of this rule.

### BS-entry-4: What new commands take: four-digit years, amounts a line can carry (B-28)

- **Status:** CONVENTION
- **Statement:** A command MUST take only dates in 0001-01-01 through 9999-12-31 written `YYYY-MM-DD`, and amounts no larger than one quadrillion with at most ten decimal places; anything else MUST be refused, never wrapped or truncated. Replaying books already written MUST NOT apply these bounds.
- **Sources:**
  - external: ISO 8601-1:2019 section 5.2.2.1, calendar dates with four-digit years in the basic profile
- **Symbolic form:** `date in [0001-01-01, 9999-12-31]`; `|amount| <= 10^15`; `scale(amount) <= 10`.
- **Examples:** none yet. The example schema writes dates as `YYYY-MM-DD` already (a malformed date fails the schema), so a refusal example needs a command whose input can be out of range; the first edition has none.
- **Applicability:** commands only; replay of existing books is exempt (posted is forever).

### BS-entry-5: Drafts and evidence (B-43)

- **Status:** reserved
- **Statement:** Reserved: allocated in the wanderbooks plan (8.0.3) for the draft states of unit P4, not yet stated in `docs/spec/rules.md`. Its identifier and area are fixed so a later edition fills it in without moving another number.
- **Sources:**
  - none yet
- **Symbolic form:** none yet.
- **Examples:** none yet
- **Applicability:** none yet.
