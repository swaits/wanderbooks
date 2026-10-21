# bookspec: examples (seed)

bookspec is the formal accounting specification planned in
`plan/served.md` (P13, D-34): every rule stated normatively, sourced to its
authority, with a symbolic form and concrete examples, independent of any
engine. Its own repository comes later. Until then this directory holds its
first examples: P3's regression tests, each written here as it is built, so
the spec starts from cases a real engine got wrong.

Every example is checked against wanderbooks on every `cargo test`
(`crates/ledger-core/tests/bookspec.rs`, and for the formats appendix
`crates/ledger-package/tests/package.rs`): an example the engine disagrees
with fails the build, so the examples can't rot. An expected value is worked
by hand from the cited source, never copied from an engine's output.

## The shape of an example

One TOML file per example in `examples/`, named for its `id`.

```toml
id = "cash-flow.reversed-property-contribution"   # stable; never reused
title = "A property contribution reversed the same day"
rule = "One normative sentence: what MUST (or MUST NOT) hold."
status = "AUTHORITY"            # AUTHORITY | CONVENTION | CHOICE
sources = ["ASC 230-10-45-24", "ASC 230-10-50-3"]   # exact citations
tags = ["accrual", "book", "us"]                     # applicability

[setup]
currency = "USD"                # ISO 4217
fiscal_year_start = "01-01"     # MM-DD
basis = "accrual"               # accrual | cash
rounding = "half-even"          # half-even | half-away-from-zero |
                                # largest-remainder: how a figure is
                                # rounded to the currency's minor unit,
                                # where a rule rounds (none here)

[[setup.accounts]]
code = "1000"
name = "Cash"
type = "asset"                  # asset | liability | equity | revenue | expense

[[events]]                      # in order; event N is entry #N
date = "2026-01-10"
description = "Property contributed"
lines = [
  { account = "1500", side = "debit", amount = "2340.00" },
  { account = "3000", side = "credit", amount = "2340.00" },
]

[[events]]
date = "2026-01-10"
description = "Reverse entry 1"
reverses = 1                    # the event (1-based) this one reverses:
                                # its lines, sides flipped; no `lines` here

[[expect]]
query = "cash_flow"             # what is asked of the books (see below)
from = "2026-01-01"             # inclusive
to = "2026-12-31"               # inclusive (a query as of a day: as_of)

[expect.values]                 # every value an exact decimal string
operating = "0.00"
```

Events count from 1, as entries do: the first `[[events]]` is event 1
(entry #1), and `reverses = N` reverses the Nth event, one before it;
`reverses = 0` names no event and is refused. An event is an entry
(`lines`), a reversal (`reverses`), a close (`close`, below) or a bill
(`bill`, below), exactly one of them, and posts entry #N. A change that
posts no entry (a rename, a policy) is a `[[changes]]` entry instead
(below), so the numbering holds. A key the shape doesn't have is refused
too, never ignored.

`rounding` names the rule the example's figures are rounded by where one
rounds: `half-even` or `half-away-from-zero` to the currency's minor unit,
or `largest-remainder` for an amount split among parts (each part rounded
down, the units left over to the largest remainders, ties to the part
listed first; rules.md B-13). An example that closes to the members
declares `largest-remainder`, the rule a members close splits by.

Required everywhere: the top-level keys, `[setup]` with its four keys and
its accounts, the events, and at least one `[[expect]]` with values: a
figure query's are exact decimal strings (`"2340.00"`, `"-12.50"`; never a
float), a format query's are text (below). Amounts on lines are positive;
the side gives the direction.

### Optional keys, for the rules that need them

- `appendix = "formats"` (top level): the example shows an interchange
  format, not an accounting rule (P13's appendix of formats; the IIF
  file today). It loads like any other; only it may ask a format query.
- `parent` on an account: its parent account's code. A parent is declared
  before its subaccounts.
- `header = true` on an account: a header, which groups other accounts
  and takes no postings (it needs no tax category: rules.md B-32).
- `tax_category` on an account: its tax category, a key of the books'
  catalog (`gross_receipts`, `office_expense`, ...; `ledger_core::tax`),
  its link to a line of the return (B-32).
- `today` (in `[setup]`): the day the books are asked on, `YYYY-MM-DD`,
  in the company's time zone: what an omitted date means (B-30). A query
  that omits a date needs it.
- `cash_accounts`, `financing_accounts`, `investing_accounts`: arrays of
  account codes, the statement of cash flows' account roles (ASC
  230-10-45-10 to 45-15). The cash and financing accounts are also what a
  payee is paid from, for `vendor_1099_paid` (a payment card is neither:
  its charges go on Form 1099-K; rules.md B-19), and the cash accounts
  are an IIF file's bank accounts (B-34).
- `[[setup.fixed_assets]]` `{ cost_account, accumulated_account,
  expense_account }`: accounts that hold property and equipment (their
  cost and accumulated depreciation are investing; ASC 230-10-45-12(c),
  45-13(c)).
- `[[setup.members]]` `{ name, capital_account, share? }`: the members of
  a multi-member LLC. `share` is the member's share of income, an exact
  decimal fraction (`"0.5"`, `"0.3333"`); every member has one or none
  does, and the shares sum to 1. A close to the members and
  `members_equity` need them.
- `[[setup.payees]]` `{ name, form }`: a payee marked for an information
  return, `form` `"1099-NEC"` or `"1099-MISC"`; each name once.
- `payee` on an event: the payee (by name, one of `setup.payees`) the
  entry paid. Not on a reversal, which takes its original's, nor on a
  close.
- `close = { start, to, rounding? }` as an event: closes the period from
  `start` through the event's `date` (rules.md B-11, B-13), `to`
  `"members"` (split by share, in the order listed) or
  `"retained_earnings"`. A close to the members splits cumulatively, each
  close giving each member their split of everything the members' closes
  have closed less what earlier closes gave them; `rounding = "per-close"`
  splits that close's net on its own instead (books from an engine that
  rounded each close by itself). A close must have something to close: it
  posts entry #N like any other event.
- `bill = { vendor, amount }` as an event: a bill from `vendor` (a name;
  a payee's record if one has it) for `amount`, dated and due on the
  event's date. It posts entry #N: General Expenses (6999) debited and
  Accounts Payable (2000) credited, each account created with that fixed
  name when first needed, as the books create a control role's account
  (B-32).
- `[[changes]]` `{ after, rename?, require_tax_category? }`: a change
  that posts no entry, exactly one of `rename = { code, name }` (rename
  the account `code`, B-31) and `require_tax_category = "warn"` or
  `"refuse"` (the policy, B-32; the books start at `warn`). It is made
  right after event `after` (1 to the number of events), once the
  expectations asked after that event are checked, in the order listed.
- `after = N` on an expectation: asked of the books after event N (before
  the changes made after it), not after the last event and every change
  (1 to the number of events; not on a format query).

### Queries and their values

A query over a period takes `from` and `to`; a query as of a day takes
`as_of`; the other is refused. A date left out is resolved by the one
rule (rules.md B-30) on `setup.today`: an omitted `to` is today, an
omitted `from` the first day of the fiscal year that holds `to`, an
omitted `as_of` today (`vendor_1099_paid` and the format queries give
theirs). A query no crate answers is refused, never skipped.

| query | dates | values | checked by |
|---|---|---|---|
| `cash_flow` | `from`, `to` | figures (below) | `ledger-core` `tests/bookspec.rs` |
| `trial_balance` | `as_of` | each account's balance by code, debits less credits (a credit balance negative), every account listed | `ledger-core` `tests/bookspec.rs` |
| `profit_and_loss` | `from`, `to` | each revenue and expense account's activity by code, on its normal side, closing entries left out, every one listed; `net_income`, revenue less expenses | `ledger-core` `tests/bookspec.rs` |
| `entry` | `entry = N`, no dates | entry N's lines, `"<code>.debit"` and `"<code>.credit"` each the sum of that account's lines on that side, every line of the entry named | `ledger-core` `tests/bookspec.rs` |
| `balances` | `as_of` | each account named by code, its balance on its normal side (a member's capital after a loss is negative); the example names the accounts it shows | `ledger-core` `tests/bookspec.rs` |
| `members_equity` | `from`, `to` | the statement of changes in members' equity (rules.md B-13): `"<member>.beginning"`, `.contributions`, `.net_income`, `.distributions`, `.other`, `.ending`; the example names the figures it shows | `ledger-core` `tests/bookspec.rs` |
| `vendor_1099_paid` | `from` (January 1 of `to`'s year), `to` | each payee's payments by name, out of the cash and financing accounts (`cash_accounts` required), every payee listed | `ledger-core` `tests/bookspec.rs` |
| `period` | `from`, `to`, either or both left out | the dates a period report asked with those dates covers, by the one rule (B-30): `from`, `to` (`YYYY-MM-DD`); or `refused` | `ledger-core` `tests/bookspec.rs` |
| `day` | `as_of`, or left out | the day a report as of a day is made as of: `as_of` | `ledger-core` `tests/bookspec.rs` |
| `calendar_year` | none | the year a calendar-year count (1099s, party totals) asked without one counts: `year` | `ledger-core` `tests/bookspec.rs` |
| `account_names` | none | each account's name by code, as the chart holds it and every report prints it, past periods included (B-31); every account listed, those the books created included | `ledger-core` `tests/bookspec.rs` |
| `uncategorized_accounts` | none | a list: every account that needs a tax category (revenue or expense, not a header) and has none, with activity or not, `"<code> <name>"`, by code (B-32) | `ledger-core` `tests/bookspec.rs` |
| `iif_lines` | `as_of` | formats appendix: the file's lines in order, each its fields joined by tabs, without the CR LF that ends each | `ledger-package` `the_bookspec_iif_example_matches_the_engine` |
| `iif_changes` | `as_of` | formats appendix: each text the file writes differently from the books, in file order, as `[what, books' text, file's text]` (the example's header gives `what`) | `ledger-package` `the_bookspec_iif_example_matches_the_engine` |

A figure query is a table (`[expect.values]`, or `values = { ... }`); a
format query, and `uncategorized_accounts`, an array (`values = [ ...
]`). `period`, `day` and `calendar_year` give dates and a year, and
`account_names` names, not figures.

A command is tried on a copy of the books, as the books' own command
does it, and changes nothing checked after it. It shows no values: it is
`accepted = true`, or `refused = "<reason>"`.

| command | keys | |
|---|---|---|
| `rename_account` | `code`, `name` | rename the account (B-31); accepted, it must move no amount and keep the name trimmed |
| `add_account` | `account = { code, name, type, parent?, header?, tax_category? }` | add the account, under the policy in force (B-31, B-32) |
| `set_tax_category` | `code`, `category` (left out: clear it) | set or clear the account's tax category, under the policy in force (B-32) |

A refusal (a command's, or a `period`'s) is named for what was refused,
never by an engine's message: `end_before_start` (both dates given, the
start after the end), `start_after_today` (only a start, after today, the
end an omitted end means), `blank`, `unchanged` (the name the account has,
trimmed), `sibling_has_it` (another account under the same parent has the
name, ignoring capitals and surrounding space), `holds_a_tin` (a run of
nine digits), `not_one_line` (a line break, tab or other control
character, or past 200 characters), `code_taken`, and
`tax_category_required` (an account that needs a category left without
one under `refuse`).

`cash_flow` (rules.md B-14): the statement of cash flows for `[from, to]`.

| key | the value |
|---|---|
| `operating`, `investing`, `financing` | each section's net cash, + in |
| `net_change`, `cash_beginning`, `cash_ending` | the cash accounts' change and balances |
| `difference` | sections less the change in cash: always `"0.00"` |
| `net_income` | revenue less expenses, closing entries excluded |
| `<section>.<N>` | entry N's cash in that section (`financing.4`) |
| `<section>.<N>.on_behalf` | of it, what a member or a lender paid or received for the company, no cash account moving (constructive receipt and disbursement) |
| `<section>.count` | how many entries the section lists |
| `noncash.<N>` | entry N's noncash investing and financing disclosure |
| `noncash.count` | how many entries are disclosed |
| `reconciliation.depreciation` | depreciation added back |
| `reconciliation.investing_gain_loss` | gains and losses whose cash is investing, taken out |
| `reconciliation.noncash_through.<code>` | operating movements no cash settled, through that account |
| `reconciliation.other_noncash` | other operating movements no cash settled |
| `reconciliation.balance_change.<code>` | minus the change in an operating asset or liability |

An entry named in a `<section>.<N>` (or `.on_behalf`) or `noncash.<N>`
key must be the only such entries when `<section>.count` or
`noncash.count` is given too; a
reconciliation key not given must be zero or absent.

Every example, in any appendix, is loaded by
`crates/ledger-core/tests/bookspec.rs` (`every_bookspec_example_holds`):
its shape (a key the shape doesn't have is refused), its setup, and every
event posted. Each query is then checked by the crate that answers it (the
table above), so the shape never rots either.

## Licence and sources

The examples state rules in their own words with the paragraph cited
(FASB's Codification is copyrighted: cited, never copied; see
`docs/spec/sources/fasb-asc.md`). The examples are MIT OR Apache-2.0, as
the rest of this repository, until bookspec moves to its own repository
(planned: the spec under CC BY 4.0).
