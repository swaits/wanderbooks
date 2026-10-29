# Formal verification: the accounting laws, proved

Tests check the examples someone thought of. Proofs check every case.
wanderbooks proves the fundamental laws of double-entry bookkeeping three
ways, each covering what the others can't:

| Layer | Tool | What it proves | Scope |
|---|---|---|---|
| **Code** | [Kani](https://model-checking.github.io/kani/) (bounded model checking) | The engine's own Rust kernel is correct and never panics or overflows | Every input within the bounds: up to 4 lines, 3 accounts, amounts to 2^40; proration by days up to 3 month parts (bounds below) |
| **Laws** | [Lean 4](https://lean-lang.org) (machine-checked proof) | The laws hold for books of any size, after any sequence of commands | Unbounded: any number of entries, accounts, periods, steps |
| **Bridge** | proptest | The kernel on `Decimal` (what the engine runs) agrees with the kernel on `i64` (what Kani proves); proration's `i128` (the engine's) with its `i64` and `i32` | 2,000 random cases per property |

Run all of it with `just prove` (after `mise install`; see the end).

## The kernel

Every invariant rests on a little arithmetic: signing a line, summing
sides, grouping by account, flipping for a reversal, reading a balance on
its normal side, splitting a movement into trial-balance columns, closing
an account, plugging the last share of an allocation. That arithmetic
lives in one place, `crates/ledger-core/src/kernel.rs`. Its functions
are small, pure, and generic over an `Amount` trait with *checked*
operations. The engine calls them on `Decimal`: `JournalEntry::validate`,
`movements`, `reverse_entry`, `account_balance`, `account_movement`,
period closing, `periods::allocate`, and the trial balance. There is no
second copy of the arithmetic for proofs to drift from.

Kani compiles the same functions at `i64` (exact integer money in the
smallest unit; the four proration charge harnesses at `i32`) and
explores every input in the bounds. (Grouping by
account is proved through `movements_into`, the allocation-free core
that `movements` wraps: symbolic heap growth is beyond what the model
checker explores in reasonable time.) The Lean model in
`verify/lean` restates each function over `Int` (proration over `Nat`,
both of `prorated_through`'s branches) and proves the laws with no bounds
at all.

## The laws

Each law, and what proves it. Kani harnesses are in
`crates/ledger-core/src/proofs.rs`; Lean theorems are in
`verify/lean/Wanderbooks/`.

### Entries

1. **An entry is accepted exactly when** it has two or more lines, every
   amount is positive and in range, and debits equal credits, to the unit.
   - Kani `check_lines_is_exact` (both directions: no bad entry gets in,
     no good one is refused), `out_of_range_is_rejected`
   - Lean `Accepted`
2. **An accepted entry nets to zero.**
   - Kani `accepted_entry_nets_to_zero` · Lean `Accepted.net_zero`
3. **Grouping by account loses and invents nothing**: each account's
   movement is the net of its own lines, and the movements sum to the
   entry's net.
   - Kani `movements_are_per_account`, `movements_sum_to_the_net` ·
     Lean `sum_movements`
4. **Sums are exact and never overflow** for in-range lines.
   - Kani `sums_are_exact` (every harness also fails on any panic or
     arithmetic overflow)

### Reversals

5. **A reversal is accepted exactly when the original is**, and **the two
   together net every account to zero**.
   - Kani `reversal_mirrors`, `reversal_cancels`, `flip_negates`
   - Lean `accepted_reversal_iff`, `movement_with_reversal`

### Balances and statements

6. **Normal sides**: a line on an account's normal side raises its
   balance by the amount; the other side lowers it.
   - Kani `normal_side_increases` · Lean `balance_after_line`
7. **The trial balance balances**, and it balances *only* if the books
   net to zero: it is a complete check of the arithmetic.
   - Kani `trial_balance_columns` · Lean `trial_balance_iff`,
     `Reachable.trial_balance`, `Reachable.trial_balance_through` (as of
     any date)
8. **The accounting equation**: Assets = Liabilities + Equity + (Revenue −
   Expenses), always and as of any date.
   - Kani `normal_balance_is_signed` (the fact it rests on, every type
     and amount) and `accounting_equation` (end to end, up to three
     accounts: regrouping signed 64-bit sums is hard for a SAT solver) ·
     Lean `accounting_equation`, `Reachable.accounting_equation`,
     `Reachable.accounting_equation_through` (any number of accounts)
9. **Period statements add up**: a year's P&L is the sum of its months'.
   - Lean `statements_add_up`
10. **The statement of cash flows foots** (rules.md B-14, plan P3 defect
    1): each entry's cash is applied to the sections moving against it,
    operating, investing, financing, each up to its own movement; what a
    member or a lender paid or received for the entry, with no cash
    account moving (rule 11, D-42), is added as a constructive receipt
    and disbursement that nets to zero within the entry; so each entry's
    flows sum to its cash and what neither settled sums to zero, and
    operating + investing + financing is exactly the change in the cash
    accounts, over any entries, for any tagging whose cash tag follows
    the account. A reversal's flows, constructive ones included, are its
    original's negated (so an entry and its reversal in one period drop
    out together), and net income plus the reconciling items (gains and
    losses on investing assets, the noncash operating parts, the changes
    in operating balances) is the operating section, whenever revenue and
    expense lines are only operating or investing. The engine checks the
    same identities at runtime and refuses (`CashFlowDoesNotFoot`) rather
    than show a statement that doesn't foot.
    - Kani `entry_flows_sum_to_cash`, `entry_flows_pure_are_movements`,
      `entry_flows_are_bounded`, `no_cash_no_flow` (end to end on entries
      of up to three lines, each in any of the four places: four lines
      ran past half an hour per harness); the reversal's mirror at three
      lines in two halves, `entry_movements_of_a_reversal_mirror` (a
      reversal's cash and section movements are its original's negated)
      and `flows_of_mirrors` (`kernel::flows_of` is odd, for any
      movements three lines can make), `kernel::entry_flows` being the
      one then the other, and end to end on up to two lines,
      `entry_flows_of_a_reversal_mirror` (at three lines end to end it
      took about 14 minutes alone, 2026-10-20, and past 30 minutes at
      four jobs on a busy machine, 2026-10-21; split then, nothing
      weakened: the halves cover every three-line entry the end-to-end
      harness did), `take_is_odd_and_bounded`, `apply_cash_mirrors` (the
      parts, for any amounts), `apply_cash_spends_exactly` (how a
      section's cash splits among its kinds, operating's classes
      included), `exchanged_is_bounded` (the
      exchange inside investing and financing that is disclosed, given
      the cash in and out of the two sections),
      `constructive_nets_to_zero` and `constructive_mirrors` (what a
      member or lender paid or received for the company, either way, for
      any amounts: the parts net to zero, settle their own movements
      toward zero and no further, leave no source beside a use it could
      settle, and mirror under a reversal)
    - Lean `net_splits_by_flow`, `applyCash_conserves`,
      `applyCash_exhausts`, `applyCash_neg`, `entry_flows_sum`,
      `flows_reversal`, `constructive_nets`, `constructive_neg`,
      `constructive_sections_net` (the kernel's `constructive` modelled as
      it is written: two passes of `applyCash`; its parts by section net
      to zero), `stmt_flows_sum`, `stmt_flows_reversal` (an entry's
      statement flows, its cash flows plus any constructive triple that
      nets to zero, and mirrors for a reversal), and over them
      `cashflow_sections_sum_to_cash_change`,
      `secNet_cash_is_movement`, `operating_reconciliation_ties`,
      `operating_reconciliation_ties_all`, `cashflow_reversal_nets`;
      `Reachable.cashflow_foots` and `Reachable.cashflow_is_change_in_cash`
      (in every reachable state: the sections over the entries between two
      days are the cash accounts' ending balance less their beginning)
    - bridge `entry_flows_agree`, `apply_cash_and_exchanged_agree`,
      `constructive_agrees`;
      properties `prop_cash_flow_laws`,
      `prop_pure_entries_sections_are_movements` (5,000 cases each)

### The books over time

The Lean model (`Ledger.lean`) describes the engine as a state machine:
add an account, start a draft, post, reverse, open a period, close one,
and record how the books are kept (a setting, convention, decision, open
item or approval: one step, `record`, each record abstracted to a number).
Each step carries the checks the engine makes before it mutates. These
theorems hold in **every state reachable from empty books, by any
sequence of commands, of any length**:

11. **The invariant holds**: chart codes are unique, every posted entry is
    acceptable and on known accounts, and no draft is dated in a closed
    period. (That last one is why the store can journal a post before
    applying it: a post can never then fail on a closed period.)
    - Lean `Step.inv`, `Reachable.inv`
12. **The books always net to zero.** · Lean `Reachable.books_net_zero`
13. **Posted is forever**: posting only appends. After any sequence of
    commands the old log is a prefix of the new one. Nothing is edited,
    reordered, or removed.
    - Lean `Step.extends`, `Steps.extends`
14. **Closed periods are frozen**: once a period is closed, no sequence of
    commands adds, removes, or changes an entry dated in it, and it stays
    closed.
    - Lean `Step.frozen`, `Steps.frozen`
15. **How the books are kept changes no entry**: in the model, a record
    (a setting, convention, decision, open item or approval; a vendor's
    1099 decision or W-9 flag, a payment method, an entry's payment-method
    tag) leaves the
    chart, every posted entry, every draft and every period exactly as they
    were, and any number of them keeps a reachable state reachable. The
    model's `record` step touches only the records by definition, so the
    proof says the model is sound, not the engine; the engine's side is
    checked by `settings_replay_is_pure`, which snapshots every figure and
    entry around each record (rules.md B-29), and for the 1099 records by
    `decision_and_method_records_change_no_entry` (ledger-store
    `tests/form1099.rs`; they change the 1099 worksheet by design, never
    an entry). Settings that change how the
    same entries are presented (the fiscal year, cash flow roles, members)
    are outside this law by design. A rename (`rename_account`, P3,
    rules.md B-31) is a record too: the model's chart is `(code, type)`
    (`Ledger.lean`), names are not modelled, so a rename is the identity
    on the model and needs no new `Step` (plan I-13 is met as P1 met it
    for records). The engine's side is the property
    `prop_rename_changes_no_figure` (ledger-store `tests/store.rs`: every
    statement amount, entry and earlier journal line unchanged, live
    equals replay, the projection unchanged but `accounts.name`), and the
    stateful model `tests/model.rs` keeps a name per account so engine
    and model agree on every rename's refusal and nothing else moves.
    - Lean `Step.record_changes_no_figure`, `Reachable.records` (no
      axioms at all) · property `settings_replay_is_pure` ·
      `prop_rename_changes_no_figure` · model `Cmd::Rename` · test
      `decision_and_method_records_change_no_entry`

### Closing

16. **A closing line zeroes its account**, and **a closing entry balances
    and zeroes every revenue and expense account** through the period's
    last day, whether the net goes to one account or is split among
    members.
    - Kani `closing_line_zeroes`, `credit_line_credits`,
      `closing_entry_balances`
    - Lean `closingEntry_net`, `closingEntry_zeroes`,
      `closingEntry_accepted`, `close_zeroes`
17. **A close credits equity with exactly the net income**: revenue minus
    expenses through the period's last day.
    - Lean `net_closings_is_net_income`, `close_credits_net_income`
18. **An allocation's parts sum to the whole, to the unit**, however the
    other shares were rounded, and only the last share changes; and the
    largest-remainder apportionment (`kernel::apportion`, rules.md B-13)
    succeeds exactly when the rounded-down parts are short of the whole by
    less than a unit each, then sums to the whole, gives each part nothing
    or one unit (one part may also take the sub-unit fraction), and never
    gives a unit to a part while one with a strictly larger remainder goes
    without (no member favored by position).
    - Kani `plug_last_sums_exactly`, `apportion_is_exact`,
      `apportion_is_fair` · Lean `plugLast_total`, `plugLast_keeps`,
      `Apportion.ranks_perm`, `Apportion.apportion_sum`,
      `Apportion.apportion_sum_of_shortfall` (any number of parts)
19. **A run of member closes telescopes**: each member's total from a run
    of cumulative closes is their allocation of everything the run closed
    (so its rounding is one allocation's, never a sum of them), and each
    close hands out exactly its net, for any allocation whose parts sum to
    the whole (rules.md B-13). Since P3 this is the run rule closes
    journaled `"cumulative"` replay under (`CloseTo::MembersRun`).
    - Lean `runPart_telescopes`, `closePart_total` · property
      `each_member_stays_within_a_cent_of_their_share`

The P3 odd cent (defect 17; rules.md B-13; D-10, D-38) adds three laws:

20. **A cumulative members close hands out exactly its net, and leaves each
    member's running total one allocation of everything the count closed**,
    whatever rule the count's earlier closes were made under, as long as
    they handed out what they closed. The run rule is the special case where
    they gave exactly the allocation. A refusal leaves the parts as they
    were.
    - Kani `apportion_cumulative_is_exact`,
      `cumulative_closing_entry_balances` (the closing entry nets to zero
      whatever the true-up)
    - Lean `trueUp_total`, `trueUp_running`, `closePart_eq_trueUp`,
      `Apportion.allocW_sum`, `Apportion.allocW_cumulative_total`
    - properties `apportion_cumulative_agrees`,
      `allocate_cumulative_agrees_with_the_integer_model`
21. **Bounded drift: no member's running total is ever a minor unit from
    their exact share.** With shares w_i / W (exact decimals) and every
    amount a whole number of minor units (B-3), after every cumulative
    close |(given_i + part_i)·W − T·w_i| < W, whatever earlier closes gave.
    The precondition is the code's: `periods::allocate_cumulative` refuses a
    product Decimal would round, so the remainders it ranks are exact, and
    they sum to whole units (the Lean hypothesis of `rem_pos_of_rank_lt`).
    - Kani `cumulative_close_never_drifts_a_unit_at_halves`,
      `cumulative_close_never_drifts_a_unit_at_tenths` and
      `cumulative_close_never_drifts_a_unit_at_near_thirds` (two members at 50/50, three at 20/30/50%, three
      at 33.33/33.33/33.34%; every running total, net and earlier split to
      2^12 units)
    - Lean `Apportion.rem_pos_of_rank_lt` (a member given a unit had a
      positive remainder), `Apportion.apportion_drift` (one apportionment
      is within a unit), `Apportion.allocW_drift` (the model of
      `allocate_cumulative`, gain or loss), `cumulative_drift` (any
      allocation within a unit stays so under the true-up),
      `Apportion.allocW_cumulative_drift` (the two together): any number of
      members, any amounts
    - property `members_capital_never_drifts_a_cent_from_their_share`
    - the tax-basis K-1's running split (B-36 with D-38, 2026-10-22),
      whose splits each carry their own weights, so the exact running
      share is a sum of quotients and Decimal's division can leave it a
      hair under the cent it is (`tax_capital::RunningSplit` takes it to
      ten places first): property `a_running_split_never_drifts_a_cent`
      (`ledger-core/tests/tax_capital.rs`, checked against the
      largest-remainder rounding of exact integer shares, and a loss
      against its gain); its kernel step is `apportion_cumulative`,
      proved above. The worksheet keeps one per set of members with a
      part in an entry, in date order whatever the span
      (`tax_capital::entry_parts`; tests
      `a_member_outside_an_entry_never_takes_its_cents`,
      `an_entrys_odd_cent_is_the_same_in_every_span`,
      `the_split_runs_in_date_order`)
22. **A member's capital on a day never moves when its period is closed.**
    The statement of members' equity gives each member, of the earnings not
    yet closed, what the count's next close would give them; closing moves
    those earnings into capital without changing the sum. When the count's
    earlier closes gave something else (closes under an earlier rule), the
    figure moves by exactly the true-up the close posts, never before.
    - Lean `statement_after_close`, `statement_unmoved_by_close`
    - property `each_members_capital_chains_and_holds_through_closes`;
      `three_members_at_a_third_each_over_twelve_months` (every day of a
      year, closed and never closed), the viewer's
      `a_members_page_shows_the_same_capital_before_and_after_a_close`

### Parties

23. **A party's part of an entry is signed as its own account would be,
    with a direction that is its sign**: paid exactly when what counts is
    positive, received when negative, neither exactly when nothing counts;
    the money and property in it each carry the whole's sign and never
    exceed it; a money target is all cash; a contribution counts all of
    itself; a distribution counts no more money than it paid and no more
    property than it gave, the rest still owed (rules.md B-35).
    - Kani `party_target_direction_is_its_sign` · bridge
      `party_target_agrees`
24. **A reversal is the exact opposite of what it reverses**: the target
    is odd (negating the lines' capital movement, money and own-account
    movement gives the original's reverse, but for the debits fallback and
    a member's entry whose cash isn't exactly its money: an expense booked
    straight to capital, a debt converted, a distribution still owed; the
    engine never reads those off a reversal's lines: its reversal arm
    takes the original's target), reversing twice gives the original
    back, and a reversal's movement over any accounts is the original's
    negated (B-9).
    - Kani `party_target_is_odd`, `reverse_target_is_an_involution` ·
      Lean `partyTarget_odd`, `capitalTarget_odd` (with `cashIsMoney`
      shown satisfiable), `reverseTarget_involutive`,
      `sumOver_movement_reversal` · property
      `a_reversal_is_the_exact_opposite`

### 1099 payments

25. **A payment to a vendor is split, never lost or counted twice**:
    each payment line goes in the bucket rules.md B-19's table names (its
    16 cases: with no payment method naming its account, a financing line
    never counts and a card account's line is always a card's); the
    buckets are each exactly the credits minus debits of their lines; and
    netting money moved between the company's own sources keeps the
    total, leaves no bucket opposite it, and leaves buckets of one sign
    alone.
    - Kani `payment_bucket_is_the_table`, `payment_split_is_exact`,
      `net_buckets_keeps_the_total` · bridge `payment_buckets_agree` ·
      test `payment_bucket_is_the_table_in_all_16_cases` · properties
      `buckets_partition_each_payment`,
      `no_instruments_no_financing_is_the_old_report`,
      `a_reversal_nets_every_bucket_to_zero` (ledger-core
      `tests/form1099.rs`)

### Proration by days, catching up, truing up (P3, defects 22 and 9)

Depreciation (rules.md B-15) and daily prepaid and deferred-revenue
schedules (B-16) spread a total over a span of days with one kernel
function, `kernel::prorate_by_days` (each month's share through
`kernel::prorated_through`), in exact integer arithmetic over a common
unit, `MONTH_L` = lcm(28, 29, 30, 31) = 377,580 parts of a month, so a day
of any month is a whole number of them.

26. **The parts of a span sum to its total exactly, none is negative, and
    the last is the total less the others** (plan/served.md P3 defect 9:
    "parts sum to the total; the last part absorbs the remainder"). Every
    split whose parts but the last fit in the span is computed; only parts
    no calendar has are refused.
    - Kani `prorate_parts_sum_to_total`, `prorate_last_part_absorbs_the_remainder` (a
      corollary, kept because the plan names the law) · Lean
      `Prorate.kernel_parts_total`, `Prorate.kernel_parts_nonneg` (the
      kernel's split, any number of parts and any sizes), `Prorate.cum_dvd`
      (a total in whole units is charged, so split, in whole units), from
      `Prorate.prorate_total`, `Prorate.prorate_nonneg`,
      `Prorate.parts_telescope` · bridge `prorate_agrees_at_i64_and_i128`
27. **Each part but the last is its days times the monthly rate within one
    minor unit** (P3 defect 9: "each part equals days × rate within 0.01",
    stated for any currency's minor unit). Each cumulative charge is its
    exact share rounded half up to a multiple of the unit (a tie goes up;
    the total itself when that rounds above it): Kani of the kernel at its
    bounds, Lean of both of the kernel's branches at any size
    (`roundUnit_near`, `cum_near`; `chargeMid_eq_cum`, `chargeLast_eq_cum`
    say the kernel's two formulas are the model's). Two such halves make
    the part's one unit (`part_near`, at any size, for any parts).
    - Kani `a_months_charge_is_within_half_a_cent`,
      `a_months_charge_in_tenths_is_within_half_a_cent`,
      `the_last_months_charge_is_within_half_a_cent`,
      `the_last_months_charge_in_tenths_is_within_half_a_cent` · Lean
      `Prorate.roundUnit_near`, `Prorate.cum_near`, `Prorate.part_near`,
      `Prorate.part_within_a_unit` · bridge `prorated_through_agrees_at_i32_and_i128`
28. **The charge through a day never falls, never exceeds the total, is
    continuous where one month ends and the next begins, and is the total
    on the last day**; malformed input (a negative total, no months, a
    unit that isn't positive, a part no month has, a day past the part, a
    weight outside the span) is refused, never a panic.
    - Lean `Prorate.cum_mono`, `Prorate.cum_le`, `Prorate.cum_full` (the
      model, any span), `Prorate.chargeLast_mono` and
      `Prorate.chargeLast_start` (the kernel's last month, by days, from
      where the month before ends: rounding is unchanged when an amount and
      its denominator are scaled together, `Prorate.roundUnit_scale`) · unit
      `the_charge_never_falls_and_is_continuous_over_real_calendars`
      (the kernel, exhaustively: every span of 1 to 3 months from every day
      of a common and a leap year, walked day by day, eight totals, both
      units) · Kani's four charge harnesses (bounded: never negative, never
      above the total, the total on the last day),
      `prorated_through_refuses_malformed_input` · property
      `prop_depreciation_is_monotone_and_bounded` (the engine)
29. **A catch-up run never carries the books past the schedule** at a
    month end already posted, and posts everything due through its month
    when no run comes after it (plan/served.md P3, settled in design: a
    month skipped is caught up in the month the run is made); every
    month's figure at once (`catch_ups`) is each suffix's catch-up. What
    the books carry at each month end is read entry by entry, each run and
    each reversal in its own month (`assets::depreciation_effects`), so the
    kernel's law is a law of the books' month ends, not of the runs'
    periods (a run reversed after its month ended is never run twice; a
    re-instated run never carries the books past the schedule).
    - Kani `catch_up_never_overtakes_the_schedule`,
      `catch_ups_is_every_suffixs_catch_up` · bridge
      `catch_up_and_true_up_agree` · unit `catch_ups_is_every_suffixs_catch_up`
      · property `prop_run_amounts_are_each_months_run_amount`
      · store `a_run_reversed_after_its_month_is_caught_up_in_the_reversals_month`,
      `reinstating_a_run_a_later_run_caught_up_is_refused`
30. **A disposal's true-up reaches the schedule** through its day from what
    the books carry, up or down, never both (`true_up` is `columns` of the
    gap).
    - Kani `true_up_reaches_the_target` · bridge `catch_up_and_true_up_agree`

Bounds (Kani): the split harnesses, at `i64`: up to 3 month parts, 1 to
3 months, each month 28 to 31 days with 1 to all of them in the span,
totals to 2^16 units (they also passed at 2^20, 2026-10-20), a unit of
1 or 10. The charge harnesses, at `i32` (the kernel is generic over
`Units`; `i32` so the solver can relate a product to a quotient): 1 or
2 months, any one month part and weight before it, totals to 2,048 units
(a month but the last) or 64 (the last month, whose numerator carries the
month's days), each unit in its own harness;
the malformed-input harness: totals to 2^20 either sign, months 0 to 3,
days, month lengths and k 0 to 40, a weight before of up to 4 ×
MONTH_L either sign, units -10 to 10; catch-up (and every suffix's) over
up to 3 month ends, and the true-up, at amounts to 2^40.

What the solver could not do, and what carries it instead: a harness
relating two symbolic divisions (a part against days × rate directly; a
charge never falling within a month; the charge through a month's last
day against the next month's through none of its days) ran past twenty
minutes each, at 2^20 and again at 2^16, with CaDiCaL and with Kissat,
and was stopped (2026-10-20); so did the one-division charge bound at
`i64` (a checked 64-bit product is a 128-bit multiplier to the solver).
Each such relation is nonlinear in the divisor. So Kani proves the
one-division fact at `i32` (each charge within half a unit of its exact
share: one to seven minutes a harness), the bridge ties the `i32` kernel
to the engine's `i128` one, and Lean proves every one of these relations
at any size, of the kernel's own two formulas: the model's charge is
`cum t unit V w` (the total times `w/V`, rounded half up to a multiple of
`unit`, capped at the total, and the total at `w = V`), and
`chargeMid`/`chargeLast` are `prorated_through`'s two branches written
out, proved equal to `cum` over `W` = months × MONTH_L and over `W` × the
last month's days, with the two meeting where the last month starts
(`roundUnit_scale`). The one hypothesis is the calendar's: every part
but the last ends short of the span (`e < W`), which a walk from any
in-service date keeps (B-15: a first month of at most one, then whole
months); an exhaustive test walks the kernel itself over every
1-to-3-month span of two calendar years, and the bridge and the exact
unit tests check the rest.

### An asset's basis figures (P3, defect 13)

The register's tax figures beside book value (rules.md B-25) come from
one kernel function, `kernel::basis_figures`, for every rule:

31. **Each figure follows from the rule and the facts, and none
    overflows**: a purchase depreciates from its cost and has no §723 basis
    or built-in gain; a contribution's §723 basis is the member's adjusted
    basis and its built-in gain the value less it (a loss exactly when the
    basis exceeds the value); carryover property depreciates from the
    basis, personal-use property from the lesser of value and basis (at
    most each, equal to one). · Kani `basis_figures_laws` (every rule,
    costs, bases and values within the bounds) · bridge `basis_figures_agree`
    (the `Decimal` instantiation agrees, rule by rule) · property
    `prop_basis_figures_laws`

### A project's unrecovered cost (P3, projects)

The "if repaid" figure of work done on spec (rules.md B-37 item 4) is
one kernel function, `kernel::unrecovered_cost`:

32. **Never below zero, the shortfall exactly, and no overflow**: the
    result is at least zero; it is expenses less revenue when expenses
    are at least revenue, and zero otherwise; with revenue not negative
    it is never above the expenses or zero, whichever is larger
    (expenses can be negative: a reimbursement credited to the expense
    above its cost). · Kani `unrecovered_cost_laws` (every
    revenue and expense a line could carry) · ledger-core
    `unrecovered_cost_only_for_on_spec_and_never_negative`. No Lean model:
    the function is one subtraction and a `max`.

### Documents and their control accounts (P4b S1, D-45)

**Law 34. A control account is its documents.** For a receivables or
payables account A and any day D, the account's balance on D equals the
sum over documents of `outstanding(d, D)` (the signed sum of the lines
tagged with d and dated on or before D, whatever their entries' standing:
a reversal's mirror carries the same tag and cancels) plus the legacy
untagged lines that still stand. A document never owes less than nothing
or more than its total on any day a payment path or a reversal touches
it. Rule: `docs/spec/rules.md` B-38.

- Kernel: `kernel::running_bounds(total, deltas)` and
  `kernel::bounds_kept(total, before, after)` decide, at every date in
  date order, whether 0 ≤ outstanding ≤ total holds; `Ledger::
  check_item_bounds` runs them on the documents a step touches. Kani:
  `bounds_check_matches_definition` (the check equals the definition on a
  symbolic sequence of deltas) and
  `bounds_kept_is_the_definition_on_books_in_bounds` (on books already
  within bounds, `bounds_kept` is exactly the definition over the new
  deltas), both with covers reaching a refused and an accepted sequence.
  Bridge: `crates/ledger-store/tests/bounds.rs` and the properties in
  `crates/ledger-store/tests/arap_props.rs` (`subledger_equals_control`
  after every operation, at every date; `status_is_a_function_of_journal
  _and_date`; `reversal_twice_restores_state`).
- Lean, `verify/lean/Wanderbooks/Documents.lean`: `control_agrees` and
  `control_agrees_through` (the identity, for every account and through
  every day), `Step.document_unattached` (an untagged legacy line only
  grows `unattached`, never a document), `Step.reverse_moves` and
  `reverse_twice` (a reversal moves exactly the opposite of its tagged
  lines from its date; reversing it restores them),
  `Reachable.control_is_documents` (the identity on every reachable
  book). Listed in `Audit.lean`; standard axioms only.
- Laws 35 to 37 are taken, in the order the paused branches merge, by
  S5 (`Step.markSent`: a send changes no figure), S6
  (`settle_is_exact`: a settlement's cash, discounts and fee are exact)
  and S7 (`writeOff_within_bounds`); the branches wrote 34 and are
  renumbered at merge (`plan/served.md` 8.0.2).

### Payment terms and document lines (P4b S4.1)

Payment terms (rules.md B-39) give a due date and a discount date by
calendar arithmetic in `arap::Terms`, and the two money figures they touch
come from the kernel:

33. **A discount never exceeds its stated percentage of the total, and a
    line's amount is exactly its quantity times its unit price.** The
    bound is `total × basis points / 10,000`, exact: a result times 10,000
    is the total times the basis points, and it is never above the total;
    when no whole amount can hold the exact bound, or a factor would
    overflow, the kernel refuses (None) rather than rounding. A line's
    amount is the exact product, never rounded, and None on overflow.
    Terms' own due dates (Net n, Net n EOM, due on receipt, custom; the
    9999-12 bound) are checked by `spec_documents.rs`'s
    `terms_derive_due_dates`, and the parser's round trip by
    `terms_describe_and_parse_round_trip`.
    - Kani `discount_max_is_bounded` (at i32, as the proration charge
      harnesses are: the law relates a product to a quotient) ·
      `line_amount_is_exact` (at i64) · bridge `discount_max_agrees`,
      `discount_max_i32_agrees_with_i64`, `line_amount_agrees` · tests
      `discount_bounds_and_late_reason`, `qty_times_price_must_equal_amount`.
      No Lean model: these are two arithmetic laws, not a new `Ledger`
      command (S4.1 adds fields to the existing document commands).

### Sent tracking (P4b S5)

35. **A send changes no figure.** `Ledger::mark_sent` is a command, so it
    is a `Step` in the model (AGENTS.md's formal-methods bar; the unit's
    "no Lean change" phrase was overruled by it, and says so in its As
    built block). `Documents.lean`'s `Books` gains `sends`, and
    `Step.markSent` records one on a document the lines carry, touching
    no line. Every control balance, every document's sum and what is
    unattached are therefore the same before and after, through any day,
    and `Reachable.tagged` (the control lines are all documents') covers
    the new step; the engine's side is the same statement, checked by
    `a_send_changes_no_figure` on the fixture book.
    - Lean `Documents.Step.markSent_changes_no_figure` (no axioms) ·
      `Documents.Reachable.tagged`, `Documents.Reachable.control_is_documents`
      cover the new step · tests `sends_replay_and_project`,
      `a_send_changes_no_figure`, `mark_sent_on_void_refused`,
      `mark_sent_waits_for_documents_v2`

### Settlements (P4b S6)

36. **A settlement is exact.** The applications' total is Σ cash + Σ
    discounts and the deposit is Σ cash − fee, with exact arithmetic; a
    fee above the cash, a negative amount, or arrays of different lengths
    is refused (None), never wrapped or rounded. The core's posting is a
    document step: every control line a settlement posts carries its
    document, so `Documents.control_agrees` covers it and no new `Step`
    constructor is needed.
    - Kani `settle_is_exact` (at i32, fixed arrays of three applications,
      a nonzero discount and a nonzero fee covered) · bridge
      `settle_agrees` · tests `bill_on_terms_with_discount`,
      `late_discount_needs_a_reason`,
      `discount_on_an_asset_bill_is_refused_with_the_remedy`,
      `a_fee_posts_gross_to_receivables_and_net_to_the_bank`,
      `a_settlement_names_its_lines_and_refuses_a_document_twice` · Lean
      `Documents.settle_posts_a_document_step`,
      `Documents.settle_unattached`.

### Bad debts (P4b S7)

A write-off (rules.md B-41) is `Ledger::write_off`: a new command, so a new
Lean step, and no new kernel function (its two lines are one debit and one
credit, and the bounds are the existing date-sum's).

37. **A write-off of at most what is owed leaves nothing owed below
    zero.** The step posts the document's control line credited (tagged
    with it, so it is the same sum B-38 fixed) and the amount of
    bad-debt expense debited (untagged, off the control accounts).
    `through_append` reads the write-off's own two lines through the day
    separately; `docSum_writeOffLines` gives the control line's −amount;
    `writeOff_within_bounds` then follows by arithmetic from the command's
    own premise (the amount is at most the date-sum through the day).
    `Step.writeOff_unattached` shows the step moves nothing unattached, so
    `control_agrees` and `Reachable.control_is_documents` hold for it with
    no new proof. A reversal re-instates: the same `reverse_twice` that
    covers payments (`Step.reverse` mirrors every tagged line).
    - Lean `Documents.Step.writeOff`, `Documents.Step.writeOff_unattached`,
      `Documents.through_append`, `Documents.docSum_writeOffLines`,
      `Documents.writeOff_within_bounds` (in `Audit.lean`) · tests
      `half_paid_rest_written_off_is_written_off`,
      `write_off_above_outstanding_refused`,
      `write_off_then_recovery_same_year`, and the store's
      `write_off_replays_and_projects` (live, replay and snapshot agree).
      The model's limits are the documents model's (one `Nat` id space,
      books built from empty); the cash-basis refusal is a policy on the
      books' settings, not arithmetic, and is tested, not modelled.

## What the proofs found

The engine already had 190+ tests, property tests, mutation testing,
and 11.5M fuzz executions with zero crashes. Writing the laws down
precisely still turned up three gaps in it, now closed:

- **Negative line amounts were accepted.** Only the MCP layer refused
  them. The engine now rejects them (`NegativeAmountLine`): direction is
  the side's job.
- **A huge amount could crash the engine.** Summing lines near
  `Decimal::MAX` panicked instead of returning an error. Lines are now
  limited to one quadrillion and ten decimal places
  (`AmountOutOfRange`), and the kernel's sums are checked.
- **Foreign-currency conversion could crash** the same way
  (`foreign_amount * fx_rate`). It is now a checked multiply.

And three things besides bugs:

- **A soundness bug in the verifier.** Kani reported a reversal that
  doesn't net to zero. Tracing it took a native replay of the
  counterexample (it passed) and a 15-line repro crate: Kani 0.68
  mis-models symbolic-length slice copies. See "Trusting the checker"
  below. Taking every red result seriously is the method.
- **Simpler code.** `credit_line` and `closing_line` are the same
  function. The model made that obvious, and the code now says so.
- **A load-bearing invariant, made explicit.** No draft can exist in a
  closed period (`Inv.draftsOpen`). That is why the store can journal a
  post before applying it: the closed-period check can never fail after
  the journal write. It was true by accident of the checks' design. Now
  it is a theorem, and breaking it breaks the build.

Each of these shipped past every test we had. That is the case for
formal methods here: tests sample, proofs exhaust. `AGENTS.md` makes
growing this coverage part of every change.

## Trusting the checker

A model checker is a program too. While building these proofs, Kani
reported that an entry plus its reversal could leave an account nonzero.
That was not a bookkeeping bug. Kani's own counterexample passes when
replayed natively. We narrowed it to a 15-line crate with no wanderbooks
code: Kani 0.68 (CBMC 6.11) mis-models `copy_from_slice` whenever the
length is symbolic. It is the known bug model-checking/kani#4904, closed
as a duplicate and unfixed in 0.68. It is unsound in both directions: it
can also prove false claims. So:

- **No harness copies a slice of symbolic length.** They write element by
  element (`reversal_cancels` says why), and the kernel functions under
  proof contain no bulk copies.
- **A canary watches for the fix.** `kani_symbolic_copy_canary` runs the
  faulty copy and is marked `should_panic`. When a Kani upgrade fixes the
  bug, the canary starts failing, and the workaround can go.
- **The proofs are not vacuous.** `kani::cover!` statements make Kani
  show that the interesting cases are reachable: an accepted four-line
  entry, a reversal on one account with several lines, three distinct
  accounts, a balanced three-account chart, a close to two members, a
  part-cash part-noncash investing entry, cash settling two sections at
  once, a mixed entry reversed, property contributed for capital; a split
  of three parts, a half-unit tie rounding up, a charge capped at a total
  finer than its unit, a catch-up across three month ends and distinct
  ones for every suffix, a true-up each way. `just prove` fails if any
  cover is unsatisfiable.

## What is and isn't proved

Be precise about the claims:

- **Kani is bounded.** It proves the kernel for every input up to 4 lines
  over 3 accounts and amounts to 2^40 units (the accounting equation end
  to end: up to 3 accounts; the cash flow entry harnesses: up to 3 lines,
  each tagged any of 4 ways), and the proration by days within the bounds
  above. Lean proves the laws for any size, but of a *model*. The model
  is a direct restatement of the kernel (for proration, its two formulas
  written out and proved equal to the model's charge), and the bridge
  property tests tie the `Decimal` (and `i128`) instantiation to the
  `i64` (and `i32`) one Kani checked.
- **`Decimal` itself is trusted.** `rust_decimal` is exact for these
  sizes: 96-bit integers scaled by a power of ten. With lines capped at
  one quadrillion and ten decimal places, every sum is exact while the
  total of all amounts stays below 7.9 × 10^18. Kani can't explore
  `Decimal` arithmetic directly: a single symbolic add runs for more than
  ten minutes.
- **The state machine is a model of `Ledger`'s checks**, not a proof
  about `ledger.rs` line by line. Its correspondence to the code is
  checked by review and by the end-to-end property tests in
  `tests/properties.rs` (trial balance, balance sheet, reversals,
  immutability) and the golden files.
- **Out of scope:** currency (the ledger is single-currency, enforced
  structurally), payroll, the calendar walk around
  the proration kernel (`calendar::month_parts`, `assets::depreciation_schedule`,
  `schedules::daily_plan`: property tests, `tests/properties.rs`, P3), and
  persistence (the journal store's hash chain is tested in `ledger-store`).
- **The cash flow model tags lines per entry with any function** meeting
  the engine's guarantees (the cash tag follows the account; revenue and
  expense lines are operating or investing; a reversal takes its
  original's tags); which tag the engine picks for a line (rule 4's
  disposals, the bill split, the chain root) is checked by the property
  and example tests, not proved. A line the engine splits between two
  sections is two lines in the model, the entry's net unchanged.
- **Closing entries and the cash balance.** The engine leaves closing
  entries out of the sections but not out of the cash accounts'
  balances; the two agree because a close moves only revenue, expense
  and equity (`Ledger::validate_close_to`) and cash accounts must be
  assets (rules.md B-14 rule 2). The Lean model's close step lets a
  target be any balance-sheet account, so `Reachable.cashflow_foots`
  takes any selection of entries with the cash over the same selection,
  and `Reachable.cashflow_is_change_in_cash` counts every entry, closes
  included. That a close never moves cash is checked by tests
  (`closing_entries_never_appear`, `prop_cash_flow_laws` law 7), not
  proved.

### P0 guard rails (2026-10-16)

P0 (`plan/served.md`) added no kernel arithmetic and no new law about
balances, so it added no Kani harness and no Lean theorem; nothing here
was weakened. What it added is checked by tests and properties (listed in
`docs/testing.md`, "P0 guard rails"): the journal format gate (newer is
never called corrupt, damage always is; the envelope and its hash are
frozen across formats, rules.md B-27), the server failing closed after a
panic, the import roots' lockdown (a property over generated paths, and a
component walk that refuses a folder swapped for a link; removed with the
import roots in P2 slice 5), files with full
TINs kept out of the books (B-26), and the agent text lint. Out of scope
for proof today: the format gate and the walk are I/O, and the model in
`verify/lean` stops at the ledger.

### P3: the odd cent (2026-10-20)

The cumulative members close (rules.md B-13) adds a kernel function,
`kernel::apportion_cumulative`, which Kani proves exact and refusal-safe
for up to three parts and checks against the integer model of
`periods::allocate_cumulative` for bounded drift; Lean proves the drift
bound and the true-up laws for any number of members and any amounts. Not
proved: the regime query (`Ledger::members_regime`: which closes count),
the Decimal exactness check in `allocate_cumulative`, and the journal's
`since_seq`. They are checked by the tests in `ledger-core`
`tests/odd_cent.rs`, `ledger-store` `tests/member_closes.rs` and the
old-books fixture `x_member_closes` (docs/testing.md, "P3: odd cents").
Kani times, each harness alone on a busy laptop (Kani 0.68, 2026-10-20):
`apportion_cumulative_is_exact` 257 s, `cumulative_close_never_drifts_a_unit_at_halves`
17 s, `cumulative_close_never_drifts_a_unit_at_tenths` 320 s,
`cumulative_close_never_drifts_a_unit_at_near_thirds` 449 s,
`cumulative_closing_entry_balances` 202 s (`apportion_is_exact`, for
scale: 121 s). Two shapes were too slow and were changed, not weakened in
what they prove: restating apportion's postconditions on the cumulative
parts directly ran past ten minutes, so `apportion_cumulative_is_exact`
proves the parts are apportion's less `given` on the same inputs (the
solver shares the two runs' formulas) and `apportion_is_exact` supplies
the rest; and the drift bound with symbolic weights ran past ten minutes,
so it is checked at three fixed sets of shares (the brief's fallback),
Lean carrying any weights.

## Running it

```sh
mise install   # once: Kani, Lean, and the rest, pinned in mise.toml
just prove       # 51 Kani harnesses and Lean
just prove-slow  # the other 9: the justfile's kani_slow
just prove-all   # both
```

`just prove` runs every Kani harness but the justfile's `kani_slow`, in
parallel (`-j $(nproc)`), the whole run under a 30-minute timeout; builds
the Lean proofs (seconds); and fails if any headline theorem depends on
anything beyond Lean's standard axioms (`propext`, `Quot.sound`,
`Classical.choice`). In particular, `sorryAx` fails it. `just prove-slow`
runs the slow harnesses, `$(nproc)` at a time, each under its own
30-minute timeout. Both fail if a harness they ask for isn't verified (the
count is checked), if any cover is unsatisfiable (a vacuous proof), or if
`kani_slow` names a harness `src/proofs.rs` doesn't have; a new harness
runs in `prove` until it is listed as slow. CI's weekly run does both, on
two runners (`.github/workflows/scheduled.yml`). Kani downloads its
pinned nightly and CBMC into `~/.kani` on first use.

**Why the split (2026-10-21, the P3 surface review).** With all of P3 the
suite is 55 harnesses. One `just prove` at four jobs (a CI runner's
cores), on the development laptop under heavy load from other work (a
load average near 30 on 16 cores, the run held to 12), could not finish
inside its 30 minutes: 52 of the 53 harnesses of the time were verified,
all of them proved, and the last, `entry_flows_of_a_reversal_mirror` at
three lines, was still running; alone, under the same load, it passed 30
minutes too. A run is only as safe as its slowest harness under the
machine it gets, so: that harness's claim at three lines is now proved in
two halves (`entry_movements_of_a_reversal_mirror`, `flows_of_mirrors`;
law 10) that take a minute or two each, and the end-to-end harness runs
at two lines; the eight harnesses that take the longest run in `just
prove-slow`, each capped on its own. Nothing was dropped.

Measured, four jobs: under that load, `just prove` 1,450 s (48 harnesses
then) and `just prove-slow` 1,365 s (seven then; `entry_flows_sum_to_cash`
1,228 s); on the same laptop quiet (2026-10-22, the final configuration),
`just prove` 308 s (47 harnesses, about 19 minutes of CPU, and Lean's 83
headline theorems) and `just prove-slow` 467 s (8 harnesses, the slowest
`entry_flows_sum_to_cash` at 301 s).

P4b S4.1's two harnesses were measured on 2026-10-26 (a busy machine);
the rest are the 2026-10-22 run. `discount_max_is_bounded`'s 163 s is
Kani's verification time (it runs in `prove-slow`). P4b S6 adds
`settle_is_exact` to `prove` and two Lean theorems to the audit
(`Documents.settle_posts_a_document_step`, `Documents.settle_unattached`).

Every harness, the law it proves, the recipe that runs it, and its time
in that quiet run (`prove`: each harness's verification time; `prove-slow`:
each harness's whole run, its compile of about 50 s included):

| Harness | Law | Recipe | Seconds |
|---|---|---|---:|
| `flip_negates` | 5 | prove | 0 |
| `sums_are_exact` | 4 | prove-slow | 82 |
| `check_lines_is_exact` | 1 | prove | 7 |
| `out_of_range_is_rejected` | 1 | prove | 0 |
| `accepted_entry_nets_to_zero` | 2 | prove | 15 |
| `movements_are_per_account` | 3 | prove | 12 |
| `movements_sum_to_the_net` | 3 | prove | 58 |
| `reversal_mirrors` | 5 | prove | 25 |
| `reversal_cancels` | 5 | prove | 69 |
| `normal_balance_is_signed` | 8 | prove | 0 |
| `accounting_equation` | 8 | prove-slow | 176 |
| `normal_side_increases` | 6 | prove | 0 |
| `trial_balance_columns` | 7 | prove | 62 |
| `closing_line_zeroes` | 16 | prove | 0 |
| `credit_line_credits` | 16 | prove | 0 |
| `closing_entry_balances` | 16 | prove | 11 |
| `payment_bucket_is_the_table` | 25 | prove | 0 |
| `payment_split_is_exact` | 25 | prove | 18 |
| `net_buckets_keeps_the_total` | 25 | prove | 5 |
| `plug_last_sums_exactly` | 18 | prove | 4 |
| `apportion_is_exact` | 18 | prove | 56 |
| `apportion_is_fair` | 18 | prove | 18 |
| `apportion_cumulative_is_exact` | 20 | prove | 44 |
| `cumulative_close_never_drifts_a_unit_at_halves` | 21 | prove | 11 |
| `cumulative_close_never_drifts_a_unit_at_tenths` | 21 | prove | 149 |
| `cumulative_close_never_drifts_a_unit_at_near_thirds` | 21 | prove-slow | 216 |
| `cumulative_closing_entry_balances` | 20 | prove-slow | 150 |
| `take_is_odd_and_bounded` | 10 | prove | 1 |
| `apply_cash_mirrors` | 10 | prove | 12 |
| `entry_flows_sum_to_cash` | 10 | prove-slow | 301 |
| `entry_flows_pure_are_movements` | 10 | prove-slow | 151 |
| `entry_flows_are_bounded` | 10 | prove | 20 |
| `entry_flows_of_a_reversal_mirror` | 10 | prove | 36 |
| `entry_movements_of_a_reversal_mirror` | 10 | prove | 30 |
| `flows_of_mirrors` | 10 | prove | 56 |
| `no_cash_no_flow` | 10 | prove | 75 |
| `apply_cash_spends_exactly` | 10 | prove | 36 |
| `exchanged_is_bounded` | 10 | prove | 0 |
| `constructive_nets_to_zero` | 10 | prove | 26 |
| `constructive_mirrors` | 10 | prove | 52 |
| `party_target_direction_is_its_sign` | 23 | prove | 2 |
| `party_target_is_odd` | 24 | prove | 5 |
| `reverse_target_is_an_involution` | 24 | prove | 0 |
| `prorate_parts_sum_to_total` | 26 | prove-slow | 222 |
| `prorate_last_part_absorbs_the_remainder` | 26 | prove | 33 |
| `a_months_charge_is_within_half_a_cent` | 27 | prove | 48 |
| `a_months_charge_in_tenths_is_within_half_a_cent` | 27 | prove | 42 |
| `the_last_months_charge_is_within_half_a_cent` | 27 | prove-slow | 148 |
| `the_last_months_charge_in_tenths_is_within_half_a_cent` | 27 | prove | 36 |
| `prorated_through_refuses_malformed_input` | 28 | prove | 7 |
| `catch_up_never_overtakes_the_schedule` | 29 | prove | 8 |
| `catch_ups_is_every_suffixs_catch_up` | 29 | prove | 35 |
| `true_up_reaches_the_target` | 30 | prove | 0 |
| `basis_figures_laws` | 31 | prove | 0 |
| `unrecovered_cost_laws` | 32 | prove | 0 |
| `discount_max_is_bounded` | 33 | prove-slow | 163 |
| `line_amount_is_exact` | 33 | prove | 6 |
| `bounds_check_matches_definition` | 34 | prove | (not in the 2026-10-22 run; P4b S1, 2026-10-24) |
| `bounds_kept_is_the_definition_on_books_in_bounds` | 34 | prove | (as above) |
| `settle_is_exact` | 36 | prove | 1 |
| `kani_symbolic_copy_canary` | (the checker: "Trusting the checker") | prove | 3 |
