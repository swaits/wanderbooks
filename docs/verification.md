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

### The books over time

The Lean model (`Ledger.lean`) describes the engine as a state machine:
add an account, start a draft, post, reverse, open a period, close one,
and record how the books are kept (a setting, convention, decision, open
item or approval: one step, `record`, each record abstracted to a number).
Each step carries the checks the engine makes before it mutates. These
theorems hold in **every state reachable from empty books, by any
sequence of commands, of any length**:

10. **The invariant holds**: chart codes are unique, every posted entry is
    acceptable and on known accounts, and no draft is dated in a closed
    period. (That last one is why the store can journal a post before
    applying it: a post can never then fail on a closed period.)
    - Lean `Step.inv`, `Reachable.inv`
11. **The books always net to zero.** · Lean `Reachable.books_net_zero`
12. **Posted is forever**: posting only appends. After any sequence of
    commands the old log is a prefix of the new one. Nothing is edited,
    reordered, or removed.
    - Lean `Step.extends`, `Steps.extends`
13. **Closed periods are frozen**: once a period is closed, no sequence of
    commands adds, removes, or changes an entry dated in it, and it stays
    closed.
    - Lean `Step.frozen`, `Steps.frozen`
13a. **How the books are kept changes no entry**: in the model, a record
    (a setting, convention, decision, open item or approval) leaves the
    chart, every posted entry, every draft and every period exactly as they
    were, and any number of them keeps a reachable state reachable. The
    model's `record` step touches only the records by definition, so the
    proof says the model is sound, not the engine; the engine's side is
    checked by `settings_replay_is_pure`, which snapshots every figure and
    entry around each record (rules.md B-29). Settings that change how the
    same entries are presented (the fiscal year, cash flow roles, members)
    are outside this law by design.
    - Lean `Step.record_changes_no_figure`, `Reachable.records` (no
      axioms at all) · property `settings_replay_is_pure`

### Closing

14. **A closing line zeroes its account**, and **a closing entry balances
    and zeroes every revenue and expense account** through the period's
    last day, whether the net goes to one account or is split among
    members.
    - Kani `closing_line_zeroes`, `credit_line_credits`,
      `closing_entry_balances`
    - Lean `closingEntry_net`, `closingEntry_zeroes`,
      `closingEntry_accepted`, `close_zeroes`
15. **A close credits equity with exactly the net income**: revenue minus
    expenses through the period's last day.
    - Lean `net_closings_is_net_income`, `close_credits_net_income`
16. **An allocation's parts sum to the whole, to the unit**, however the
    other shares were rounded, and only the last share changes.
    - Kani `plug_last_sums_exactly` · Lean `plugLast_total`,
      `plugLast_keeps`
17. **A run of member closes telescopes**: each member's total from a run
    of cumulative closes is their allocation of everything the run closed
    (so its rounding is one allocation's, never a sum of them), and each
    close hands out exactly its net, for any allocation whose parts sum to
    the whole (rules.md B-13).
    - Lean `runPart_telescopes`, `closePart_total` · property
      `each_member_stays_within_a_cent_of_their_share`

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
  accounts, a balanced three-account chart, a close to two members. `just prove` fails if any cover is unsatisfiable.

## What is and isn't proved

Be precise about the claims:

- **Kani is bounded.** It proves the kernel for every input up to 4 lines
  over 3 accounts and amounts to 2^40 units (the accounting equation end
  to end: up to 3 accounts). Lean proves the laws for any
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

### P0 guard rails (2026-10-16)

P0 (`plan/served.md`) added no kernel arithmetic and no new law about
balances, so it added no Kani harness and no Lean theorem; nothing here
was weakened. What it added is checked by tests and properties (listed in
`docs/testing.md`, "P0 guard rails"): the journal format gate (newer is
never called corrupt, damage always is; the envelope and its hash are
frozen across formats, rules.md B-27), the server failing closed after a
panic, the import roots' lockdown (a property over generated paths, and a
component walk that refuses a folder swapped for a link), files with full
TINs kept out of the books (B-26), and the agent text lint. Out of scope
for proof today: the format gate and the walk are I/O, and the model in
`verify/lean` stops at the ledger.

## Running it

```sh
mise install   # once: Kani, Lean, and the rest, pinned in mise.toml
just prove
```

`just prove` runs every Kani harness, builds the Lean proofs (seconds),
and fails if any headline theorem depends on anything beyond Lean's
standard axioms (`propext`, `Quot.sound`, `Classical.choice`). In
particular, `sorryAx` fails it. Kani downloads its pinned nightly and
CBMC into `~/.kani` on first use.
