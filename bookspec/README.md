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

[[events]]                      # in order, counting from 1
date = "2026-01-10"
description = "Property contributed"
lines = [
  { account = "1500", side = "debit", amount = "2340.00" },
  { account = "3000", side = "credit", amount = "2340.00" },
]

[[events]]
date = "2026-01-10"
description = "Reverse entry 1"
reverses = 1                    # the event (1-based) whose entry this
                                # one reverses: its lines, sides flipped;
                                # no `lines` here

[[expect]]
query = "cash_flow"             # what is asked of the books (see below)
from = "2026-01-01"             # inclusive
to = "2026-12-31"               # inclusive (a query as of a day: as_of)

[expect.values]                 # every value an exact decimal string
operating = "0.00"
```

Events count from 1: the first `[[events]]` is event 1. An event is an
entry (`lines`), a reversal (`reverses`), a close (`close`, below), a
bill or an invoice (`bill`, `invoice`, below), each posting one entry;
or one of the books' commands that post what is due (`run_depreciation`, `dispose_asset`,
`run_schedules`, below), which post as many entries as are due, none
included; or a schedule's registration (`register_schedule`), which
posts none: exactly one of them. A key that names an event (`reverses =
N`, `entry = N`, a cash flow's `<N>`) names the entry event N posted, one
before it: an event that posted none, or several, can't be named that
way, and `reverses = 0` names no event and is refused. While every event
posts one entry, event N's is entry #N. A change that posts no entry (a
rename, a policy) is a `[[changes]]` entry instead (below). A key the
shape doesn't have is refused too, never ignored.

An entry, a reversal, a close, a bill, an invoice and a disposal are
dated (`date`); the other commands date their own entries (a run its
month's end) and take no date. An entry, a reversal, a close, a bill and
an invoice are described (`description`); a command describes its own
entries and takes none.

The books are the engine's own (`ledger_store::Store`, in a directory of
their own): each event is the command the books take it by, journaled as
they journal it, and after every expectation is checked the books must
replay to what they kept (`Store::verify`).

`rounding` names the rule the example's figures are rounded by where one
rounds: `half-even` or `half-away-from-zero` to the currency's minor unit,
or `largest-remainder` for an amount split among parts (each part rounded
down, the units left over to the largest remainders, ties to the part
listed first; rules.md B-13). An example that closes to the members
declares `largest-remainder`, the rule a members close splits by.

`status` says what the expected values rest on: `AUTHORITY`, the cited
authorities alone (a standard, a regulation, a statute, a court, the
mechanics of double entry); `CONVENTION`, common practice where no
authority governs (rules.md's CONVENTION); `CHOICE`, an owner's or the
house's decision (a plan D-number, or a DECISION rules.md records) that
some expected value depends on, even when authorities carry the rest. An
example whose figures follow a decision cites it in `sources`.

Required everywhere: the top-level keys (`title`, `rule`, `tags` and
`sources` non-empty), `[setup]` with its four keys and its accounts, the
events (`events = []`, before `[setup]`, when nothing is posted), and at
least one `[[expect]]`. A value from a closed list (a type, a kind, a
role, a form, a basis) is spelled exactly as written here. A figure
query's values are
exact decimal strings (`"2340.00"`, `"-12.50"`; never a float), a format
query's are text (below). Amounts on lines are positive;
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
- `cash_accounts`, `financing_accounts`, `investing_accounts` (in
  `[setup]`): arrays of account codes, the statement of cash flows'
  account roles (ASC 230-10-45-10 to 45-15). The cash and financing
  accounts are also what a payee is paid from, for `vendor_1099_paid` and
  `vendor_1099` (a payment card is neither: its charges go on Form
  1099-K; rules.md B-19); with the cards, what party money is, for
  `party_part` and `party_totals` (B-35 item 5: no `cash_accounts`, every
  asset account but receivables); the cash accounts are what the K-1
  worksheet reads as money (`k1`, `k1_tax_basis`, `k1_reconciliation`)
  and an IIF file's bank accounts (B-34).
- `[[setup.fixed_assets]]` `{ cost_account, accumulated_account,
  expense_account }`: accounts that hold property and equipment (their
  cost and accumulated depreciation are investing; ASC 230-10-45-12(c),
  45-13(c)), for an example no asset's own figures matter to. An asset
  with its figures is a `[[setup.assets]]` entry (below), whose accounts
  hold property and equipment too.
- `[[setup.assets]]`: fixed assets registered before the first event,
  each named by its `id` in the events and queries that ask about it
  (rules.md B-15):

  ```toml
  [[setup.assets]]
  id = "X"                      # its name in this example
  description = "Asset X"
  cost = "12000.00"
  salvage = "0.00"
  life_months = 12
  in_service = "2026-01-01"     # depreciation starts here
  method = "straight_line"      # the one method the books have
  cost_account = "1500"
  accumulated_account = "1510"
  expense_account = "6200"
  acquired_by = 2               # optional: the event whose entry put it
                                # on the books (B-15 rule 7)
  basis = { rule = "personal_use_conversion",
            contributor_adjusted_basis = "1688.40",
            fmv_at_contribution = "1440.00",
            sec704c_method = "traditional" }   # optional (B-25)
  ```

  Registering an asset posts nothing: the entry that buys it is an event
  of its own. `acquired_by = N` links that entry, event N's, once the
  event is applied: its date is the asset's acquired date, which puts it
  on the register (before it, the in-service date does). `basis` records
  how the company got it and the facts its tax figures follow from, as
  the asset is registered: `rule` `"purchase"`, `"carryover_723"` or
  `"personal_use_conversion"`; a contribution's
  `contributor_adjusted_basis` and `fmv_at_contribution`; the §704(c)
  method if chosen (`"traditional"`, `"traditional_with_curative"`,
  `"remedial"`).
- `[[setup.schedules]]`, and `register_schedule = { ... }` as an event: a
  prepaid expense or deferred revenue recognized over its months (rules.md
  B-16), named by its `id`:
  `{ id, kind, description, total, start, months, convention,
  balance_account, pl_account }`, `kind` `"prepaid_expense"` or
  `"deferred_revenue"`, `convention` `"daily"` (by the day, the last month
  taking what the others leave) or `"monthly_even"` (equal months from
  the 1st). Registering one posts nothing.
- `[[setup.parties]]` `{ name, roles, creditor_of?, tin_last4?,
  w9_tin_last4?, tin_purged? }`: the company's directory (B-26). `roles`
  are role keys (`member`, `vendor`, `customer`, `contractor`,
  `landlord`, `employee`, `financial_institution`, `payout_platform`,
  `tax_agency`, `government_agency`); `vendor` or `customer` (one at
  most) gives the party its record. `creditor_of`: the liabilities, by
  code, the company owes the party itself (a card's issuer, a lender,
  each member for what the members advanced: B-35 item 6). `tin_last4`:
  its TIN facts' last four (an SSN's); `w9_tin_last4`: its W-9's (a 1099
  role's); `tin_purged = true`: the vault's copy purged, the facts kept (needs
  `tin_last4`).
  Never a full TIN. Each name once.
- `[[setup.members]]` `{ name, capital_account, share?,
  distributions_account? }`: the members of a multi-member LLC. `share`
  is the member's share of income, an exact decimal fraction above zero (`"0.5"`,
  `"0.3333"`); every member has one or none does, and the shares sum to
  1. A close to the members, `members_equity`, `k1`, `k1_tax_basis` and
  `k1_reconciliation` need them. `distributions_account`: the member's
  distributions account (B-13). A
  member whose `name` is a party's in `setup.parties` with the `member`
  role is that party, as the company's settings name them: an entry's
  parties' shares resolve with those members (B-35), and so does the
  1099 worksheet.
- `[[setup.payees]]` `{ name, form }`: a payee marked for an information
  return from the beginning, `form` `"1099-NEC"` or `"1099-MISC"`; each
  name once. A payee in `setup.parties` with the `vendor` role is that
  party's vendor record; any other is a vendor record of its own.
- `[[setup.status_changes]]` `{ payee, effective, decision, reason_kind?,
  reason? }`: a payee's 1099 decision from `effective` (`YYYY-MM-DD`) on
  (rules.md B-19, D-40: an LLC's corporate election, say): `decision`
  `"1099-NEC"`, `"1099-MISC"`, `"undetermined"` or `"not_reportable"`,
  the last with `reason_kind` (`corporation`, `tax_exempt`,
  `united_states`, `state`, `foreign_government`, `foreign_person`,
  `merchandise`, `employee`, `other`) and the facts in words (`reason`).
  Each payment takes the decision in force on its date.
- `[[setup.instruments]]` `{ label, kind, account, last4?, owner? }`: a
  payment method (B-19): `kind` `bank`, `business_card`, `member_card`,
  `member_cash_or_check` or `platform`; `account` the code it pays
  through; `owner` a party's name in `setup.parties` (informational); each label
  once. A business card's
  liability is a card account (B-35 item 5): while none is recorded, a
  liability whose name says "card", not "gift card", is.
- `[[setup.projects]]` `{ name, kind?, status?, party? }`: a project
  (rules.md B-37), added with its facts, each name once: `kind`
  `internal`, `client`, `on_spec` or `other`; `status` `active` (the
  default), `paused`, `finished` or `abandoned`; `party` a
  `setup.parties` name.
- `project = "<name>"` on a line of `lines`: the line is tagged with
  that project (a `setup.projects` name), as the books' command tags it,
  after the line is added. Not on a reversal, which carries its
  original's tags.
- `payee` on an event: the payee (by name, one of `setup.payees`) the
  entry paid. Only on an entry (`lines`): a reversal takes its
  original's, a bill names its vendor, an invoice its customer, a close
  and a command have none.
- `parties = [{ party, amount? }, { party, percent? }, ...]` on an event:
  the parties (by name, in `setup.parties`) the entry is with, each with
  its share: an `amount` (an exact decimal string), a `percent`, or
  neither (derived from the entry: all of it for one party, the members'
  capital lines or income shares for several: B-35). An entry naming a
  party with a vendor record is tagged with that payee too; with
  `payee` as well, the payee's party is one of the parties.
- `instrument` on an event: the payment method (a `setup.instruments`
  label) the entry was paid with. Neither `parties` nor `instrument` on a
  reversal, which takes its original's, nor on a command.
- `close = { start, to, rounding? }` as an event: closes the period from
  `start` through the event's `date` (rules.md B-11, B-13), `to`
  `"members"` (split by share, in the order listed) or
  `"retained_earnings"`. A close to the members splits cumulatively, each
  close giving each member their split of everything the members' closes
  have closed less what earlier closes gave them; `rounding = "per-close"` (members only)
  splits that close's net on its own instead (books from an engine that
  rounded each close by itself). A close must have something to close: it
  posts one entry like any other event.
- `bill = { vendor, amount }` or `bill = { vendor, lines = [{ amount,
  description?, project? }, ...] }` as an event: a bill from `vendor` (a
  name; a payee's record if one has it) for `amount` (one line, described
  as the event) or its `lines` (exactly one of them; a line's description
  is the event's when left out), dated and due on the event's date. It
  posts one entry: General Expenses (6999) debited and Accounts Payable
  (2000) credited, each account created with that fixed name when first
  needed, as the books create a control role's account (B-32). A line's
  `project` (a `setup.projects` name) puts its cost on that project: the
  entry's expense side is one line per project named, the untagged lines'
  as one, the payable line never tagged (B-37 item 8).
- `invoice = { customer, amount }` or `invoice = { customer, lines = [...]
  }` as an event: an invoice to `customer` (a name, its record added when
  first named), as a bill is (the same `lines`), dated and due on the
  event's date. It posts one entry: Accounts Receivable (1200) debited and
  Revenue (4000) credited, each created with that fixed name when first
  needed; a line's `project` puts its revenue on that project, the
  receivable line never tagged (B-37 item 8).
- `run_depreciation = { asset, period }` as an event: the asset's run for
  `period` (`"YYYY-MM"`), which posts what the schedule says is due
  through the month's end and the books don't carry, dated then: a
  skipped month caught up, nothing when nothing is due (rules.md B-15).
- `dispose_asset = { asset, proceeds, proceeds_account, gain_loss_account
  }` as an event, dated: the asset's disposal for `proceeds` (an exact
  decimal, zero allowed), received into `proceeds_account`, its gain or
  loss to `gain_loss_account` (B-15). It posts the depreciation still due
  through its day in an entry of its own when the month has no run, then
  the disposal entry.
- `run_schedules = { through }` as an event: every schedule's months due
  through `through` posted (B-16).
- `refused = "<reason>"` on a reversal or a command: the books refuse it,
  for that reason (below); it changes nothing, posts nothing, and the
  example goes on without it.
- `[[changes]]` `{ after, rename?, require_tax_category?, correct_asset?
  }`: a change that posts no entry, exactly one of `rename = { code, name
  }` (rename the account `code`, B-31), `require_tax_category = "warn"`
  or `"refuse"` (the policy, B-32; the books start at `warn`) and
  `correct_asset = { asset, cost?, salvage?, life_months?, in_service?,
  reason }` (a registration mistake corrected: each figure given replaces
  the asset's, the rest stay; refused while the books carry depreciation
  for it). It is made
  right after event `after` (1 to the number of events), once the
  expectations asked after that event are checked, in the order listed.
- `after = N` on an expectation: asked of the books after event N (before
  the changes made after it), not after the last event and every change
  (1 to the number of events; not on a format query).

Party totals and the 1099 worksheet answer different questions (B-35
item 9): a member paying a contractor through the members' financing
account is, in party totals, paid to the contractor (money, the
members' financing being money) and nothing to the member until the
company reimburses them; on the 1099 worksheet it is the company's
payment, counted toward the contractor's 1099 unless it was made by card
(B-19), and undetermined until its payment method is recorded.

### Queries and their values

A query over a period takes `from` and `to`; a query as of a day takes
`as_of`; the other is refused. A date left out is resolved by the one
rule (rules.md B-30) on `setup.today`: an omitted `to` is today, an
omitted `from` the first day of the fiscal year that holds `to`, an
omitted `as_of` today (`vendor_1099_paid`, `vendor_1099`,
`party_totals` and the format queries give theirs). A query no crate
answers is refused, never skipped.

| query | dates | values | checked by |
|---|---|---|---|
| `cash_flow` | `from`, `to` | figures (below) | `ledger-core` `tests/bookspec.rs` |
| `trial_balance` | `as_of` | each account's balance by code, debits less credits (a credit balance negative), every account of `setup.accounts` listed (not those the books create) | `ledger-core` `tests/bookspec.rs` |
| `profit_and_loss` | `from`, `to` | each revenue and expense account's activity by code, on its normal side, closing entries left out, every one of `setup.accounts` listed; `net_income`, revenue less expenses | `ledger-core` `tests/bookspec.rs` |
| `entry` | `entry = N`, no dates | the lines of event N's entry, `"<code>.debit"` and `"<code>.credit"` each the sum of that account's lines on that side, every line of the entry named | `ledger-core` `tests/bookspec.rs` |
| `balances` | `as_of` | each account named by code, its balance on its normal side (a member's capital after a loss is negative); the example names the accounts it shows | `ledger-core` `tests/bookspec.rs` |
| `members_equity` | `from`, `to` | the statement of changes in members' equity (rules.md B-13): `"<member>.beginning"`, `.contributions`, `.net_income`, `.distributions`, `.other`, `.ending`; the example names the figures it shows | `ledger-core` `tests/bookspec.rs` |
| `k1` | `from`, `to` | the K-1 worksheet (B-13; Schedule K-1 Item L): `"<member>.beginning"`, `.contributions_cash`, `.contributions_property`, `.net_income`, `.distributions`, `.other`, `.ending`; the example names the figures it shows (`cash_accounts` required) | `ledger-core` `tests/bookspec.rs` |
| `k1_tax_basis` | `from`, `to` | the K-1 worksheet on the tax basis (B-36; Schedule K-1 Item L, tax-basis method), beside `k1`'s book figures: `"<member>.beginning"`, `.contributions_cash`, `.contributions_property`, `.net_income`, `.other`, `.distributions`, `.ending`; a line the books can't know is `"open"`, with what they know so far as `"<member>.known_<line>"`; the example names the figures it shows (`cash_accounts` required) | `ledger-core` `tests/bookspec.rs` |
| `k1_reconciliation` | `from`, `to` | each member's book capital less tax-basis capital at `to`, by cause (B-36): `"<member>.beginning"`, `.built_in_gain_704c`, `.property_without_basis`, `.property_distributed`, `.total`; the example names the members it shows, each with every cause (`cash_accounts` required) | `ledger-core` `tests/bookspec.rs` |
| `vendor_1099_paid` | `from` (January 1 of `to`'s year), `to` | each payee's payments that count toward its 1099, by name: out of the cash and financing accounts (`cash_accounts` required), a card's or payment network's left out (B-19), every payee listed | `ledger-core` `tests/bookspec.rs` |
| `vendor_1099` | `payee`; `from` (January 1 of `to`'s year), `to` | one payee's line of the 1099 worksheet (B-19): `status` (`reportable`, `under_threshold`, `not_reportable` or `undetermined`), `threshold`, `paid` (counts toward its 1099), `excluded_card` (a card's or payment network's: Form 1099-K), `undetermined` (through a financing account, its payment method not recorded), `not_reportable` (paid while decided not reportable), every one given | `ledger-core` `tests/bookspec.rs` |
| `party_part` | `entry = N`, `party`, no dates | entry N's part for the party (B-35): `resolved`, signed as a line on the party's own account would be (paid or given positive, received negative), the entry's `direction` (`paid`, `received` or `neither`), and what of it moved as money (`cash`) and as property (`in_kind`), every one given | `ledger-core` `tests/bookspec.rs` |
| `party_totals` | `party`, `basis` (`cash` or `all`), `from`, `to` | what was `paid` to the party and `received` from it over the period (B-35 item 4: basis `cash` money, `all` money and property in kind; each entry on its first original's side), both given | `ledger-core` `tests/bookspec.rs` |
| `party_tin_last4` | `party`, no dates | `tin_last4`: the party's TIN facts' last four, else its W-9's, `""` for none (B-26) | `ledger-core` `tests/bookspec.rs` |
| `period` | `from`, `to`, either or both left out | the dates a period report asked with those dates covers, by the one rule (B-30): `from`, `to` (`YYYY-MM-DD`); or `refused` | `ledger-core` `tests/bookspec.rs` |
| `day` | `as_of`, or left out | the day a report as of a day is made as of: `as_of` | `ledger-core` `tests/bookspec.rs` |
| `calendar_year` | none | the year a calendar-year count (1099s, party totals) asked without one counts: `year` | `ledger-core` `tests/bookspec.rs` |
| `account_names` | none | each account's name by code, as the chart holds it and every report prints it, past periods included (B-31); every account listed, those the books created included | `ledger-core` `tests/bookspec.rs` |
| `uncategorized_accounts` | none | a list: every account that needs a tax category (revenue or expense, not a header) and has none, with activity or not, `"<code> <name>"`, by code (B-32) | `ledger-core` `tests/bookspec.rs` |
| `run` | `event = N`, no dates | what the `run_depreciation` event N posted: `amount` (zero when nothing was due, or the month had its run already), and `caught_up`, what of it is beyond the month's own depreciation (B-15); both listed | `ledger-core` `tests/bookspec.rs` |
| `disposal` | `asset`, no dates | what the asset's disposal took (B-15), every figure listed: `final_depreciation` (posted in its own entry), `catch_up_depreciation` and `excess_depreciation` (charged, and credited back, in the disposal entry), `accumulated` (cleared), `book_value`, `gain_loss` (a gain positive) | `ledger-core` `tests/bookspec.rs` |
| `depreciation_schedule` | `asset`, `as_of` | the asset's schedule through the day (B-15): each month named (`"YYYY-MM"`) its depreciation, `"0.00"` for a month it doesn't reach; `accumulated` and `book_value` through the day; the example names the figures it shows | `ledger-core` `tests/bookspec.rs` |
| `register_row` | `asset`, `as_of` | the asset's row of the fixed asset register (B-15 rules 7 and 8, B-24): `acquired`, `in_service` (`YYYY-MM-DD`), `cost`, `accumulated_scheduled` (the schedule's), `accumulated_posted` (what the books carry), `unposted` (the one less the other), `book_value` (cost less posted), each `"0.00"` once disposed; the example names the figures it shows; an asset acquired after the day is refused | `ledger-core` `tests/bookspec.rs` |
| `register_totals` | `as_of` | the register's totals over the assets held (B-24, B-25): `total_cost`, `total_accumulated_scheduled`, `total_accumulated_posted`, `total_book_value`, `total_depreciable_basis`, `total_tax_basis_723`, `missing_basis` (a count), and how it ties to the books: `unregistered_cost`, `unposted_depreciation`, `unattributed_accumulated`; the example names the figures it shows | `ledger-core` `tests/bookspec.rs` |
| `asset_basis` | `asset`, no dates | the asset's basis facts and the tax figures they give (B-25): `rule`, `contributor_adjusted_basis`, `fmv_at_contribution`, `tax_basis_723`, `depreciable_basis`, `built_in_gain_704c` (a loss negative), each `"none"` where it has none, and `sec704c_1c` (`"true"` for a built-in loss, which counts only for the contributor); the example names the figures it shows | `ledger-core` `tests/bookspec.rs` |
| `project` | `project`, `from`, `to` | the project's figures (B-37), every one given: `revenue`, `expenses` and `net` for the period (the revenue and expense lines tagged to it, closes left out), `to_date.revenue`, `to_date.expenses` and `to_date.net` (every tagged line through `to`), and `unrecovered_cost` (expenses less revenue to date, never below zero; `"none"` unless it is on spec) | `ledger-core` `tests/bookspec.rs`; `ledger-mcp` `project_figures_match_profit_and_loss_by_class` |
| `schedule_plan` | `schedule`, no dates | the schedule's plan (B-16): what it recognizes at each month end named (`"YYYY-MM-DD"`), `"0.00"` at one it doesn't post; `total`, all it recognizes; the example names the figures it shows | `ledger-core` `tests/bookspec.rs` |
| `iif_lines` | `as_of` | formats appendix: the file's lines in order, each its fields joined by tabs, without the CR LF that ends each | `ledger-package` `the_bookspec_iif_example_matches_the_engine` |
| `iif_changes` | `as_of` | formats appendix: each text the file writes differently from the books, in file order, as `[what, books' text, file's text]` (the example's header gives `what`) | `ledger-package` `the_bookspec_iif_example_matches_the_engine` |

A figure query is a table (`[expect.values]`, or `values = { ... }`); a
format query, and `uncategorized_accounts`, an array (`values = [ ...
]`). `period`, `day` and `calendar_year` give dates and a year,
`account_names` names, `party_part`'s `direction`, `vendor_1099`'s
`status` and `party_tin_last4` words, not figures; `register_row` gives
dates beside its figures, `project` gives `"none"` as the unrecovered cost of a
project that isn't on spec, `asset_basis` a rule's name, `"none"` and
`"true"` or `"false"`, and a `k1_tax_basis` line `"open"`, beside them.

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
one under `refuse`). A refused event's: `depreciation_past_disposal` (a
disposal dated before depreciation a run carries: a later month's run
stands, or a run's change dated after the disposal's month isn't undone),
`entry_irreversible` (an asset's entry that can't be reversed: its
disposal, a disposed asset's depreciation, a run re-instated twice or past
its schedule, or a reversal dated before its run), `already_disposed`,
`invalid_depreciation_period` (a month that ends before the asset is in
service) and `invalid_schedule` (an equal-months schedule not starting on
the 1st, among others).

`cash_flow` (rules.md B-14): the statement of cash flows for `[from, to]`.

| key | the value |
|---|---|
| `operating`, `investing`, `financing` | each section's net cash, + in |
| `net_change`, `cash_beginning`, `cash_ending` | the cash accounts' change and balances |
| `difference` | sections less the change in cash: always `"0.00"` |
| `net_income` | revenue less expenses, closing entries excluded |
| `<section>.<N>` | event N's entry's cash in that section (`financing.4`) |
| `<section>.<N>.on_behalf` | of it, what a member or a lender paid or received for the company, no cash account moving (constructive receipt and disbursement) |
| `<section>.count` | how many entries the section lists |
| `class.<a..g>` | an operating class's total, by ASC 230-10-45-25's letter: (a) from customers, (b) interest and dividends received, (c) other receipts, (d) paid to suppliers and employees, (e) interest paid, (f) income taxes paid, (g) other payments; given one, every class the statement has must be given |
| `noncash.<N>` | event N's entry's noncash investing and financing disclosure |
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
event taken or refused as it says, on the books' own store. Each query is
then checked by the crate that answers it (the table above), so the shape
never rots either.

## Licence and sources

The examples state rules in their own words with the paragraph cited
(FASB's Codification is copyrighted: cited, never copied; see
`docs/spec/sources/fasb-asc.md`). The examples are MIT OR Apache-2.0, as
the rest of this repository, until bookspec moves to its own repository
(planned: the spec under CC BY 4.0).
