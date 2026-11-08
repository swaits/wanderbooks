# Claims: period

Wanderbooks' claims for bookspec area **period**: the engine's pick for each rule, where it enforces it, and which tests check it. The accounting text lives in bookspec's records (`rules/period/`).

Rules: B-11, B-12, B-30.

---

## B-11 Revenue and expenses close into profit and loss, and that into capital

**bookspec:** `BS-period-1`

**Claims:** yes

**Enforced:** `Ledger::close_period_to` (INVARIANT 5: a close with
nothing to close posts no entry and still closes the period; the EM's
decision, D-73, 2026-11-08). **Checked:** Kani
`closing_line_zeroes`, `closing_entry_balances`; Lean
`closingEntry_zeroes`, `closingEntry_accepted`, `closingEntry_net`,
`close_zeroes`, `close_credits_net_income`;
`a_close_sweeps_only_through_the_period_end`,
`a_close_with_nothing_to_close_posts_no_entry`.

## B-12 A closed period takes no new postings; periods close in order

**bookspec:** `BS-period-2`

**Pick (BS-period-4):** nest-never-partially-overlap, the owner, 2026-10-13

**Claims:** yes

**Pick:** periods nest and never partially overlap (the owner, 2026-10-13).

**Enforced:** `closed_period_containing`,
`CloseOutOfOrder`, `Ledger::create_period` (`PeriodsOverlap`),
`Store::rehearse`. **Checked:** Lean `Steps.frozen`;
store property `refused_commands_never_reach_the_journal` (no draft in a
closed period); `periods_nest_but_never_partially_overlap`
(`crates/ledger-core/tests/invariants.rs`) and `periods_nest`
(`crates/ledger-core/tests/spec_ledger.rs`).

### Product rules

**WB-period-1** A refused command never reaches the journal: the store
rehearses a command against the books before journaling it, so a posting
refused for a closed period (or any other refusal) leaves the journal
untouched.

**Enforced:** `Store::rehearse`. **Checked:** store property
`refused_commands_never_reach_the_journal`.

## B-30 A report asked for without dates covers this fiscal year so far

**bookspec:** `BS-period-3`

**Pick (BS-period-5):** fiscal-year-to-date-by-default, the owner, 2026-10-20

**Claims:** partly (the bookspec record states the one default rule; the surfaces it binds are wanderbooks product, below)

**Pick:** a report asked for without dates covers this fiscal year so far, everywhere (the owner, 2026-10-20, `plan/served.md` P3 "Settled in design": "a report asked for without dates covers this fiscal year so far, everywhere, and an account's register opens on it with 'Show all dates' one click away").

**Enforced:** `ledger_core::calendar` (`period_defaults`,
`PeriodDefaults::statement`, `statement_range`, `as_of`,
`calendar_year`, `fiscal_year_start_on`, `PeriodKind::describe`,
`START_DOC`, `END_DOC`, `AS_OF_DOC`, `YEAR_DOC`), whose refusals are
`LedgerError::InvalidDateRange` (both dates given, start after end) and
`LedgerError::StartAfterToday` (only a start, after today, the end an
omitted end would mean; the message says the end was defaulted: pass
end); a resolved start after the resolved end is never swapped or
clamped to fit. `ledger_package`
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

### Product rules

**WB-period-2** The one default (WB pick above, BS-period-3) binds every
wanderbooks surface by name: the agent's report tools, `books.show`, the
viewer and the tool descriptions must not say different things. The
books' annual period is the fiscal year (`company.fiscal_year_start`,
B-29) and "today" is the company's (`company.timezone`, B-29).

**Enforced:** `ledger-mcp` `BooksServer::periods`, every report tool,
`books.show`, generated descriptions; `ledger-gui` `Books::periods`,
`default_period`. **Checked:** `ledger-mcp`
`period_tools_describe_the_one_default`; `ledger-cli`
`one_default_period_everywhere_defect_16`.

**WB-period-3** The CPA package's period and the viewer's "Year to date"
preset are the fiscal year that holds their day
(`calendar::fiscal_year_start_on`), the fact itself, never "what an
omitted date means": they do not follow the BS-period-3 default if it
ever changes.

**Enforced:** `ledger_package` `build_package`; `ledger-gui`
`fiscal_ytd`. **Checked:** `ledger-cli`
`the_default_period_never_shortens_the_iif_or_the_package`.

**WB-period-4** Not covered by the default (no default period): search
filters, the activity digest (its `since` is required; the
`books://digest` resource reads this month so far), the `through` of
`books.run_schedules` and `books.run_recurring` (today: an operation, not
a period), and the viewer's drill-downs from a balance, which run from
the books' first entry because a balance is cumulative. The IIF is never
a period (B-34): it refuses `start` and `end`, so the general journal's
default period never shortens it, nor anything in the CPA package, which
gives its statements and ledgers its own fiscal year to date.

**Enforced:** `ledger_package` `export_iif`, `build_package`;
`ledger-mcp` digest resource. **Checked:** `ledger-cli`
`the_default_period_never_shortens_the_iif_or_the_package`.
