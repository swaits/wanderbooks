# bookspec, first edition: definitions and the model

bookspec is a specification of double-entry bookkeeping that stands apart
from any program. Each rule is stated normatively, sourced to an authority
(or marked as a convention or a choice), written symbolically, and shown
by concrete examples (`bookspec/examples/*.toml`) that any engine can be
run against (`bookspec/runner`). wanderbooks is the first engine checked;
the GnuCash oracle is the second (`bookspec/gnucash`).

## Reading the rules

The words MUST, MUST NOT and MAY are RFC 2119's. A rule has

- an **identifier**, `BS-<area>-<n>`, stable and never reused; the table
  from the wanderbooks catalogue's `B-1` to `B-45` is `rules-index.md`;
- a **status**: `AUTHORITY` (a source states it), `CONVENTION` (common
  practice no authority states; the best reference is given), or `CHOICE`
  (a person decided; the specification names the options and an engine
  declares which it picks, `Claims::choices`);
- a **statement** (what MUST hold), its **sources** (each pinned exactly:
  a vendored source with a verbatim quotation the build checks, or an
  external reference to a paragraph, never a paraphrase of memory), its
  **symbolic form**, its **examples** (ids of the TOML files) and its
  **applicability** (which entities, bases and jurisdictions).

A rule with no example is stated and not claimable: an engine can only
claim a rule an example demonstrates (`covered_rules`). The first edition
says plainly which rules those are (`rules-index.md`, "—").

Sources are US GAAP (the FASB Codification: cited, never copied, and read
only in the Foundation's own view or a publisher that reproduces it) and
US federal tax (the Internal Revenue Code, Treasury Regulations, IRS
publications and form instructions, which are public domain), then older
or general authorities (Pacioli, OpenStax, ISO 4217). The large firms'
guides are never a source.

## The model

- **Currency and money.** A ledger has one **base currency**. An
  **amount** is an exact decimal, a whole number of the currency's
  smallest unit (ISO 4217 minor units). Arithmetic on amounts is exact;
  it never uses binary floating point, and it rounds only where a rule
  says so, and then by the rounding mode the rule names.
- **Account.** A code, a name and one of five **types**: asset,
  liability, equity, revenue, expense. An account may have a parent of
  its own type; an account may be a **header** (it groups, and takes no
  posting). Assets and expenses have a **normal balance** on the debit
  side; liabilities, equity and revenue on the credit side.
- **Line.** `(account, side, amount)`: `side` is debit or credit,
  `amount > 0`. Its **signed value** is `+amount` for a debit and
  `-amount` for a credit.
- **Entry.** A dated, described list of at least two lines. An entry is
  **posted** or not; the ledger holds the posted ones. A **reversal** of
  an entry is a new entry with the same accounts and amounts and the
  sides flipped, linked to it.
- **Ledger.** The sequence of posted entries, written only forward. The
  **balance** of an account through a day is the sum of its lines' signed
  values dated on or before the day; shown on its normal side it is that
  sum, negated for a credit-normal account.
- **Period and close.** A **period** is a range of days. **Closing** it
  posts an entry that brings every revenue and expense account's balance
  through the period's last day to zero against equity.
- **Document.** An invoice or a bill: a receivable or payable the ledger
  carries as lines tagged with the document; control accounts equal the
  documents behind them.

## The five laws

Every other rule serves these, and each is a theorem in `bookspec/lean`
(`Bookspec.law_*`), true of every state the ledger can reach by any
sequence of its commands:

1. **Debits equal credits.** `sum over all posted lines of signed value = 0`
   (`law_debits_equal_credits`; BS-entry-1).
2. **The accounting equation.** `A = L + E + (R - X)`, with each term the
   sum of the balances of that type's accounts on their normal side
   (`law_accounting_equation`; BS-statement-2).
3. **The trial balance balances.** The debit balances sum to the credit
   balances (`law_trial_balance`; BS-statement-1).
4. **Net income articulates to equity.** Closing a period credits equity
   with exactly `R - X` through its last day
   (`law_net_income_articulates`; BS-period-1).
5. **The cash flow articulates to cash.** The three sections of the
   statement of cash flows sum to the change in the cash accounts'
   balances (`law_cash_flow_is_change_in_cash`; BS-statement-3).

Money in the proofs is `Int`, a count of the smallest unit; every law is
a law of addition, and any finite set of exact decimals scales to
integers without loss.

## How an example is checked

An example is a TOML file: a setup, a sequence of events, and
expectations asked of the books, with every expected figure worked by
hand from the rule and its sources (`bookspec/README.md` has the shape).
Two checks stand behind each: **conformance** (the runner drives an
engine through it and compares the engine's answers with the expected
figures) and **derivation** (the checker computes the expected figures
from the rule's symbolic form, independent of any engine, and fails the
specification's own build when the example's figure disagrees; see
`bookspec/checker/src/derive.rs`).
