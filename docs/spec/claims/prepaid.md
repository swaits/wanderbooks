# Claims: prepaid

Wanderbooks' claims for bookspec area **prepaid**: what each rule requires, where the engine enforces it, and which tests check it. Being reduced to the engine's pick and its evidence as P14 unit S-D lands; the accounting text moves to bookspec's `spec/prepaid.md`.

Rules: B-16.

---

## B-16 Prepaid expenses and deferred revenue are recognized over the period benefited or earned

**bookspec:** `BS-prepaid-1`

**Status:** AUTHORITY (revenue); CONVENTION (prepaids)

ASC 606-10-45-2 (cited): payment before transfer → "the entity shall
present the contract as a contract liability"; 606-10-25-23: "An entity
shall recognize revenue when (or as) the entity satisfies a performance
obligation by transferring a promised good or service." Prepaids: OpenStax
§4.2 (cited); ASC 340-10-05-4 and 210-10-45-1(g), -2 (read 2026-10-14,
`sources/fasb-asc.md`): a prepaid is an asset paid in advance of its use,
current within the operating cycle; the ASC prescribes no expensing
method, so monthly expensing stays a convention. Tax: the 12-month rule, Treas. Reg. §1.263(a)-4(f)
(`sources/irs-capitalization.md`).

Conventions (P3, defect 9): `daily` or `monthly_even`. A new schedule
spreads by the day from its start (the first day covered), as
depreciation does (B-15 rules 1-4: full months equal, the first and last
partial months by their days, the last taking what the others leave,
rounded to the minor unit): a 12-month policy from March 15 posts
thirteen month ends, a small March and a small final March (CHOICE,
the owner, D-11 confirmed in D-37, 2026-10-20). Schedules registered before
P3 keep equal months, the last absorbing rounding, forever: a posted
month's replay re-derives its plan. `books.schedule_convention` sets the
convention new schedules take (unset: daily). An equal-months schedule
starts on the 1st of a month, for prepaid expenses and deferred revenue
alike (CHOICE, the owner, D-37, 2026-10-20): revenue is never recognized
before it is earned (ASC 606-10-25-23), and one rule for both is simpler
and never early; schedules an earlier version took mid-month replay as
they were. A reversed month is un-posted, so it is due again; a month in
a closed period, or one whose posting was reversed after the month ended
(posting it again on its own date would recognize it twice until the
reversal; a schedule posts its plan's months, so it can't catch one up
later), is left by a run for a human, listed with why, never a failed run.
A month's posting can't be reversed on a day before its own (a
correction comes after what it corrects, B-9; commands only). Not yet
vendored: ASC 606-10-25-31 (progress toward satisfying an obligation over
time), 606-10-25-32 (one method, applied consistently), 606-10-55-20 (time
elapsed as an input method): to read from the owner's ASC login before
B-16 cites them (P3 depreciation review R7).

**Enforced:** `schedules` (`daily_plan`, `amortization_plan`,
`validate_schedule`), `kernel::prorate_by_days`, `Ledger::reverse_entry`,
`Ledger::schedule_month_hold`.
**Checked:** `a_prepaid_expense_is_expensed_month_by_month`,
`deferred_revenue_is_earned_to_the_cent`,
`defect9_a_new_schedule_spreads_by_the_day`,
`defect9_deferred_revenue_daily_is_earned_to_the_cent`,
`defect9_legacy_schedules_stay_monthly_even`,
`an_equal_months_schedule_starts_on_the_first`,
`a_jpy_daily_schedule_posts_whole_yen`, `a_kwd_daily_schedule_rounds_to_the_fils`,
`a_reversed_schedule_month_is_due_again`,
`a_reversal_dated_before_the_entry_it_reverses_is_refused`,
`a_schedule_month_reversed_after_it_ended_is_left_for_a_human`,
`a_month_that_rounds_to_nothing_is_never_a_posting`,
`a_schedule_month_posted_again_waits_for_month_reposting_and_replays`,
`daily_plans_pin_month_end_clamping`,
`prop_daily_plan_sums_exactly_and_lands_on_month_ends`,
`prop_monthly_even_plan_is_unchanged`; `ledger-cli`
`mcp_daily_schedule`.

