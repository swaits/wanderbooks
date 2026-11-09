# Claims: prepaid

Wanderbooks' claims for bookspec area **prepaid**: the engine's pick for each rule, where it enforces it, and which tests check it. The accounting text lives in bookspec's records (`rules/prepaid/`).

Rules: B-16.

---

## B-16 Prepaid expenses and deferred revenue are recognized over the period benefited or earned

**bookspec:** `BS-prepaid-1`

**Pick (BS-prepaid-2):** `daily`, the owner, 2026-10-20, D-11 and D-37

**Pick (BS-prepaid-3):** `start-on-the-first`, the owner, 2026-10-20, D-37

**Pick (BS-prepaid-4):** `minor-unit-for-new-schedules`, the owner, 2026-11-08, D-72

**Claims:** yes

**Pick:** new schedules spread by the day (`daily`), the owner, D-11 confirmed in D-37, 2026-10-20; an equal-months schedule starts on the 1st of a month for prepaid expenses and deferred revenue alike (the owner, D-37, 2026-10-20); new monthly-even schedules round to the currency's minor unit (the owner, D-72, 2026-11-08).

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

### Product rules

**WB-prepaid-1** `books.schedule_convention` sets the convention new
schedules take (unset: `daily`, the pick above). It is a book setting,
not part of the accounting rule.

**Enforced:** `schedules`, `books.schedule_convention`. **Checked:**
`mcp_daily_schedule`, `defect9_a_new_schedule_spreads_by_the_day`.

**WB-prepaid-2** Versioning (the owner, D-72, 2026-11-08): schedules
written before the rounding change keep the rule they were written
under, so nothing journaled moves. Schedules registered before P3 keep
equal months, the last absorbing rounding, forever: a posted month's
replay re-derives its plan. Monthly-even schedules journaled before
D-72 replay the cent rule (two places for every currency); only new
schedules round to the currency's minor unit. Schedules an earlier
version took mid-month under equal months replay as they were. (The
engine change is unit K-U5's; this note records the claim.)

**Claims:** partly (the replay half is implemented; moving new monthly-even
schedules to the currency's minor unit is K-U5, not landed)

**Enforced:** `schedules::amortization_plan` (the cent rule, kept for
replay), `Ledger::replay_register_schedule`.
**Checked:** `defect9_legacy_schedules_stay_monthly_even`,
`prop_monthly_even_plan_is_unchanged`.

**Enforced:** `schedules` replay (`amortization_plan`),
`Ledger::schedule_month_hold`. **Checked:**
`defect9_legacy_schedules_stay_monthly_even`,
`prop_monthly_even_plan_is_unchanged`,
`a_schedule_month_posted_again_waits_for_month_reposting_and_replays`.
