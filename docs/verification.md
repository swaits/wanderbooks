# Formal verification: the accounting laws, proved

Tests check the examples someone thought of. Proofs check every case.
wanderbooks proves the fundamental laws of double-entry bookkeeping three
ways, each covering what the others can't:

| Layer | Tool | What it proves | Scope |
|---|---|---|---|
| **Code** | [Kani](https://model-checking.github.io/kani/) (bounded model checking) | The engine's own Rust kernel is correct and never panics or overflows | Every input within the bounds: up to 4 lines, 3 accounts, amounts to 2^40 |
| **Laws** | [Lean 4](https://lean-lang.org) (machine-checked proof) | The laws hold for books of any size, after any sequence of commands | Unbounded: any number of entries, accounts, periods, steps |
| **Bridge** | proptest | The kernel on `Decimal` (what the engine runs) agrees with the kernel on `i64` (what Kani proves) | 2,000 random cases per property |

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
smallest unit) and explores every input in the bounds. (Grouping by
account is proved through `movements_into`, the allocation-free core
that `movements` wraps: symbolic heap growth is beyond what the model
checker explores in reasonable time.) The Lean model in
`verify/lean` restates each function over `Int` and proves the laws with
no bounds at all.

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
      `entry_flows_are_bounded`, `entry_flows_of_a_reversal_mirror`,
      `no_cash_no_flow` (end to end on entries of up to three lines, each
      in any of the four places: four lines ran past half an hour per
      harness; at three, the reversal's mirror is the slowest harness in
      the suite, about 14 minutes on the development laptop, the others
      seconds to 6 minutes), `take_is_odd_and_bounded`, `apply_cash_mirrors` (the
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
14a. **How the books are kept changes no entry**: in the model, a record
    (a setting, convention, decision, open item or approval) leaves the
    chart, every posted entry, every draft and every period exactly as they
    were, and any number of them keeps a reachable state reachable. The
    model's `record` step touches only the records by definition, so the
    proof says the model is sound, not the engine; the engine's side is
    checked by `settings_replay_is_pure`, which snapshots every figure and
    entry around each record (rules.md B-29). Settings that change how the
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
      `prop_rename_changes_no_figure` · model `Cmd::Rename`

### Closing

15. **A closing line zeroes its account**, and **a closing entry balances
    and zeroes every revenue and expense account** through the period's
    last day, whether the net goes to one account or is split among
    members.
    - Kani `closing_line_zeroes`, `credit_line_credits`,
      `closing_entry_balances`
    - Lean `closingEntry_net`, `closingEntry_zeroes`,
      `closingEntry_accepted`, `close_zeroes`
16. **A close credits equity with exactly the net income**: revenue minus
    expenses through the period's last day.
    - Lean `net_closings_is_net_income`, `close_credits_net_income`
17. **An allocation's parts sum to the whole, to the unit**, however the
    other shares were rounded, and only the last share changes.
    - Kani `plug_last_sums_exactly` · Lean `plugLast_total`,
      `plugLast_keeps`
18. **A run of member closes telescopes**: each member's total from a run
    of cumulative closes is their allocation of everything the run closed
    (so its rounding is one allocation's, never a sum of them), and each
    close hands out exactly its net, for any allocation whose parts sum to
    the whole (rules.md B-13). Since P3 this is the run rule closes
    journaled `"cumulative"` replay under (`CloseTo::MembersRun`).
    - Lean `runPart_telescopes`, `closePart_total` · property
      `each_member_stays_within_a_cent_of_their_share`

The P3 odd cent (defect 17; rules.md B-13; D-10, D-38) adds three laws;
their numbers are assigned when P3 lands:

- **A cumulative members close hands out exactly its net, and leaves each
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
- **Bounded drift: no member's running total is ever a minor unit from
  their exact share.** With shares w_i / W (exact decimals) and every
  amount a whole number of minor units (B-3), after every cumulative
  close |(given_i + part_i)·W − T·w_i| < W, whatever earlier closes gave.
  The precondition is the code's: `periods::allocate_cumulative` refuses a
  product Decimal would round, so the remainders it ranks are exact, and
  they sum to whole units (the Lean hypothesis of `rem_pos_of_rank_lt`).
  - Kani `cumulative_close_never_drifts_a_unit_at_halves`, `_at_tenths`
    and `_at_near_thirds` (two members at 50/50, three at 20/30/50%, three
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
- **A member's capital on a day never moves when its period is closed.**
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

19. **A party's part of an entry is signed as its own account would be,
    with a direction that is its sign**: paid exactly when what counts is
    positive, received when negative, neither exactly when nothing counts;
    the money and property in it each carry the whole's sign and never
    exceed it; a money target is all cash; a contribution counts all of
    itself; a distribution counts no more money than it paid and no more
    property than it gave, the rest still owed (rules.md B-35).
    - Kani `party_target_direction_is_its_sign` · bridge
      `party_target_agrees`
20. **A reversal is the exact opposite of what it reverses**: the target
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
  once, a mixed entry reversed, property contributed for capital. `just
  prove` fails if any cover is unsatisfiable.

## What is and isn't proved

Be precise about the claims:

- **Kani is bounded.** It proves the kernel for every input up to 4 lines
  over 3 accounts and amounts to 2^40 units (the accounting equation end
  to end: up to 3 accounts; the cash flow entry harnesses: up to 3 lines,
  each tagged any of 4 ways). Lean proves the laws for any
  size, but of a *model*. The model is a direct restatement of the kernel,
  and the bridge property test ties the `Decimal` instantiation to the
  `i64` one Kani checked.
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
  structurally), A/R and A/P documents, payroll, depreciation schedules
  (those have their own property tests), and persistence (the journal
  store's hash chain is tested in `ledger-store`).
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
17 s, `_at_tenths` 320 s, `_at_near_thirds` 449 s,
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
just prove
```

`just prove` runs every Kani harness, in parallel (`-j $(nproc)`, under
a 30-minute timeout), builds the Lean proofs (seconds),
and fails if any headline theorem depends on anything beyond Lean's
standard axioms (`propext`, `Quot.sound`, `Classical.choice`). In
particular, `sorryAx` fails it. Its time is the slowest harness's:
`entry_flows_of_a_reversal_mirror`, about 14 minutes on the development
laptop (8 cores); the 20 harnesses before P3 take about 17 minutes of
CPU in all (`accounting_equation` the longest, about 4), the cash flow's
other ten about 17 (`constructive_mirrors` about 2.5 minutes,
`constructive_nets_to_zero` about 1.5), so the whole run is about 15 to
20 minutes. Kani
downloads its pinned nightly and CBMC into `~/.kani` on first use.
