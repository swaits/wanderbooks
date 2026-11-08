# Claims: period

Wanderbooks' claims for bookspec area **period**: what each rule requires, where the engine enforces it, and which tests check it. Being reduced to the engine's pick and its evidence as P14 unit S-D lands; the accounting text moves to bookspec's `spec/period.md`.

Rules: B-11, B-12, B-30.

---

## B-11 Revenue and expenses close into profit and loss, and that into capital

**bookspec:** `BS-period-1`

**Status:** AUTHORITY (mechanics); the account the net lands in depends on
the entity

- Pacioli ch. 34: "you shall close this account into the capital account";
  ch. 27: profit and loss "must then be transferred for its closing (saldo)
  into the capital account."
- OpenStax §5.1 (cited): "Temporary (nominal) accounts are accounts that
  are closed at the end of each accounting period." Corporations close into
  retained earnings; partnerships into each partner's capital (B-13).

**Enforced:** `Ledger::close_period_to`. **Checked:** Kani
`closing_line_zeroes`, `closing_entry_balances`; Lean
`closingEntry_zeroes`, `closingEntry_accepted`, `close_zeroes`,
`close_credits_net_income`; `a_close_sweeps_only_through_the_period_end`.

## B-12 A closed period takes no new postings; periods close in order

**bookspec:** `BS-period-2`

**Status:** CONVENTION

Periods themselves: IRS Pub. 538 (`sources/irs-periods-and-methods.md`)
defines the tax year, calendar and fiscal. Locking a closed period and
closing in order are practice (and follow from B-8: a closed period's
book is not rewritten). Periods nest (a month in a quarter in a year)
and never partially overlap: CHOICE (the owner, 2026-10-13),
`Ledger::create_period` (`PeriodsOverlap`;
`periods_nest_but_never_partially_overlap`). **Enforced:** `closed_period_containing`,
`CloseOutOfOrder`, `Store::rehearse`. **Checked:** Lean `Steps.frozen`;
store property `refused_commands_never_reach_the_journal` (no draft in a
closed period).

## B-30 A report asked for without dates covers this fiscal year so far

**bookspec:** `BS-period-3`

**Status:** AUTHORITY for the period (IRS Pub. 538,
`sources/irs-periods-and-methods.md`: "A tax year is an annual accounting
period for keeping records and reporting income and expenses"; 26 U.S.C.
§6041(a), "in any calendar year", and §6041A(a)(2), "during such calendar
year", `sources/irs-1099-thresholds.md`, for the calendar-year counts);
CHOICE for using it as the default (no authority says what a tool's
omitted dates mean; the owner, 2026-10-20, `plan/served.md` P3 "Settled in
design": "a report asked for without dates covers this fiscal year so
far, everywhere, and an account's register opens on it with 'Show all
dates' one click away").

The books' annual period is the fiscal year (`company.fiscal_year_start`,
B-29). Every place a date may be omitted resolves it by one rule
(`ledger_core::calendar::PeriodDefaults`), so the agent's report tools,
`books.show`, the viewer and the tool descriptions can't say different
things:

| Kind | Used for | Both dates omitted | Only `end` given | Only `start` given |
|---|---|---|---|---|
| A period | profit and loss (and by class), cash flow, tax summary, members' equity, general ledger, general journal (csv, json, xlsx), an account's register, `books.show` of an account or a report | the first day of the fiscal year that holds today, through today | the first day of the fiscal year that holds `end`, through `end` | `start` through today |
| An instant | balance sheet, trial balance, A/R and A/P aging, fixed asset register, the IIF's `as_of` (every posted entry from the first through it, B-34), the CPA package's `as_of` | today | | |
| A calendar year | party totals (`books.parties`, `books.party`, the viewer's party pages), the viewer's 1099 worksheet | today's calendar year | | |

- A resolved start after the resolved end is refused, whether the caller
  gave both dates (`InvalidDateRange`) or only a start after today, the
  end an omitted end means (`StartAfterToday`, whose message says the end
  was defaulted: "pass end"). It is never swapped or clamped to fit.
- A derived start before 0001-01-01 is the calendar's first day (only an
  end early in year 1 under a non-January fiscal year, B-28).
- The first year: when the company was formed after its fiscal year's
  first day, the default start comes before formation. The amounts are
  the same, and the start is not moved to `company.formed`: start-up and
  organizational costs (IRC §195, §709) can be dated before formation.
- "Today" is the company's (`company.timezone`, B-29).
- The CPA package's period and the viewer's "Year to date" preset are
  the fiscal year that holds their day (`calendar::fiscal_year_start_on`),
  the fact itself, never "what an omitted date means": they don't follow
  this default if it ever changes.
- Not covered (no default period): search filters, the activity digest
  (its `since` is required; the `books://digest` resource reads this
  month so far), the `through` of `books.run_schedules` and
  `books.run_recurring` (today: an operation, not a period), and the
  viewer's drill-downs from a balance, which run from the books' first
  entry because a balance is cumulative. The IIF is never a period (B-34):
  it refuses `start` and `end`, so the general journal's default period
  never shortens it, nor anything in the CPA package, which gives its
  statements and ledgers its own fiscal year to date.

**Enforced:** `ledger_core::calendar` (`period_defaults`,
`PeriodDefaults::statement`, `statement_range`, `as_of`,
`calendar_year`, `fiscal_year_start_on`, `PeriodKind::describe`,
`START_DOC`, `END_DOC`, `AS_OF_DOC`, `YEAR_DOC`); `ledger_package`
`statement_range`, `as_of_date`, `pnl_statement`, `bs_statement`,
`cash_flow_statement`, `build_package`; `ledger-mcp` `BooksServer::periods`,
`statement_range`, `as_of`, every report tool, `books.show`, the
generated descriptions (`described`) and parameter docs; `ledger-gui`
`Books::periods`, `default_period`, `fiscal_ytd`, `App::show`,
`RegisterRange`. **Checked:** `ledger-core` `calendar.rs` unit tests and
`tests/properties.rs` `prop_period_defaults_resolve`; `ledger-mcp`
`server.rs` `period_tools_describe_the_one_default`; `ledger-cli`
`tests/mcp_journeys.rs` `one_default_period_everywhere_defect_16` and
`the_default_period_never_shortens_the_iif_or_the_package`,
`tests/oracle_mcp.rs`
`show_sends_the_dates_it_names_defect_16`,
`show_refuses_start_after_end_defect_15`,
`show_refuses_a_derived_inverted_range_defect_15`; `ledger-gui`
`tests/show.rs`; bookspec `examples/default-period-fiscal-year-to-date.toml`.

