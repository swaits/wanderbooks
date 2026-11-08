# K-U2: one rounding primitive, `round_step`

Status: landed 2026-11-08. The kernel sweep's U2
(`plan/kernel-sweep-audit.md` section 3): every money `round_dp*`/`rescale`
in `ledger-core` goes through one kernel primitive, the law is proved once
in bookspec's Lean, and the primitive is checked on `i64` by Kani and on
`Decimal` by the bridge.

## The API

```rust
pub enum Rounding { HalfAwayFromZero, HalfToEven, TowardZero, TowardNegativeInfinity }

pub fn round_step<T: Amount>(amount: T, step: T, mode: Rounding) -> Option<T>
pub fn round_places(amount: Decimal, places: u32, mode: Rounding) -> Option<Decimal>
pub fn round_minor(amount: Decimal, currency: Currency, mode: Rounding) -> Option<Decimal>
```

`round_step` is `amount` to a whole multiple of a positive `step` in `mode`;
`None` for `step <= 0` or an unrepresentable result; never a panic and never
a negative zero. `round_places` is the behaviour-preserving call-site
wrapper: when the amount carries no more places than `places` it is returned
unchanged, otherwise the result carries exactly `places` places, matching
`Decimal::round_dp_with_strategy` to the character (including its scale).
`round_minor` picks `10^-currency.minor_units()`; a currency ISO 4217 gives
no minor unit is returned exactly, the FX rule's existing behaviour (B-3).

The `Amount` trait gained `checked_round_to_step`, implemented for
`Decimal`, `i32` and `i64` exactly as `checked_div_round_cent` was: the
`Decimal` implementation is exact integer arithmetic on mantissas and scales
(no 28-digit intermediate division), with the trailing-zero reduction that
`checked_div_round_cent` uses when the result can still be held.

## Kani harnesses (`crates/ledger-core/src/proofs/round.rs`)

All run at `i64`, amounts symbolic in `[-2^12, 2^12]`, over the fixed steps
`1, 2, 3, 7, 10, 101` (and the even steps `2, 4, 10, 100` for the tie
harness). A symbolic divisor was tried first and did not finish in 30
minutes: CBMC's bit-blasted division by a symbolic `i64` is intractable. The
fixed steps include an even step (a tie exists), odd steps, a
non-power-of-ten, a decimal step and a step well above one; the bridge's
exact-reference property carries the same laws to every `Decimal` step, and
bookspec's Lean proves them for any step.

| Harness | Law | Verification |
|---|---|---|
| `round_step_is_multiple` | the result is a whole multiple of the step; the sign never crosses zero; zero in, zero out | 5.1 s (2 covers) |
| `round_step_is_nearest` | the half modes are within half a step (a tie at exactly half) | 22 s total (3 covers) |
| `round_step_ties` | an exact tie: half away goes on, half even takes the even index, toward zero truncates, toward negative infinity floors (both signs) | 22 s total (4 covers) |
| `round_step_is_idempotent` | rounding twice is rounding once | 16 s total |
| `round_step_never_negative_zero` | zero in gives zero; a zero result comes from under a step; the sign never flips | 12 s total (1 cover) |
| `round_step_refuses_nonpositive_step` | a step of zero or less is `None` | 8 s total (1 cover) |

Measured alone on this machine, 2026-11-08, with
`cargo kani -p ledger-core --exact --harness proofs::round::<name>`.

`just prove` (fast set): **59 harnesses, 0 failures** (53 before this unit,
6 added), 440 s of Kani, 470 s wall including the Lean build (a second run,
after the last cover was added: 406 s wall); all covers satisfied.

## Bridge (`crates/ledger-core/tests/kernel_bridge/round.rs`)

- `i64_and_decimal_agree`: the Kani model in cents and the engine in dollars
  agree, value for value, for every mode.
- `decimal_matches_the_integer_reference`: an independent `i128` reference
  (integer division and remainder, sharing no code with the kernel) over
  amounts of 0 to 10 decimal places and steps 1, 0.1, 0.01, 0.001, 0.0001.
- `round_step_is_round_dp_for_a_power_of_ten`: with a power-of-ten step the
  primitive is exactly the old `round_dp_with_strategy` value.
- `round_places_matches_round_dp_to_the_character`: and the wrapper matches
  it including scale (a negative zero becomes `+0`).
- `round_step_at_the_edges`: `.5`, `-.5`, `0.004`, `1.005`, `-1.005`, a tie
  at scale 28, `Decimal::MAX`, and a non-positive step.
- `round_places_keeps_the_old_scale`, `round_minor_follows_the_currency`:
  the wrapper's early return and scale, and USD/JPY/KWD/gold minor units.

All pass with the rest of `kernel_bridge`.

## Lean (bookspec, branch `k-u2`)

`lean/Bookspec/Round.lean`, law 40 in `spec/laws.md`, headline theorems in
`lean/Audit.lean`. Build: `lake build && lake env lean Audit.lean`, standard
axioms only; `cargo test --workspace` passes (the checker's law count moves
from 39 to 40).

- `Round.roundStep_multiple` — the result is a multiple of the step.
- `Round.near_halfAway`, `Round.near_halfEven` — within half a step.
- `Round.tie_halfAway_pos` / `_neg`, `Round.tie_halfEven_pos` / `_neg`,
  `Round.tie_towardZero_pos` / `_neg`, `Round.tie_towardNegInf_pos` / `_neg`
  — each mode's tie rule, both signs.
- `Round.roundStep_idempotent` — rounding twice is rounding once.
- `Round.roundStep_odd` — the symmetric modes are odd
  (`round(-x) = -round(x)`); toward negative infinity is excluded by a
  hypothesis because it is not symmetric.
- `Round.roundStep_scale` — `round(c·x, c·step) = c·round(x, step)` for a
  positive `c`.

The half-away mode is `Deductible.roundHA`; its near, tie, odd and scale
theorems are reused, not restated. The model rounds the magnitude and puts
the sign back, so an `Int` has no negative zero.

## Call sites

Every money `round_dp*` was replaced; the remaining `rescale`s in
`ledger-core` are display padding (`money.rs` `Steps::decimal`,
`reports.rs` contribution-margin's one place), not rounding, and were left.

Counts by file (14 replaced):

| File | Sites |
|---|---|
| `ledger.rs` | 1 (`add_line_fx`, via `round_minor`) |
| `periods.rs` | 3 |
| `schedules.rs` | 1 |
| `form1099.rs` | 2 |
| `shares.rs` | 1 |
| `k1.rs` | 1 |
| `conventions.rs` | 1 |
| `tax_capital.rs` | 2 |
| `cashflow.rs` | 1 |
| `reports.rs` | 1 |

No result changes: the wrapper reproduces `round_dp_with_strategy` exactly
(value, scale and early return), the bridge checks that to the character,
and every existing test and golden passed unchanged.

### Currency-blind sites (kept at two places, on purpose)

These round to cents whatever the base currency; fixing them is another
unit's decision (D-72 for schedules), not this behaviour-preserving one:

- `schedules.rs` `amortization_plan` — the monthly-even plan's replay-frozen
  cent rule; K-U5 owns the move to the currency's minor unit (D-72).
- `form1099.rs` `vendor_part` and `split_share` — the 1099 worksheet's cents.
- `periods.rs` `allocate` and `allocate_last_absorbs` — frozen legacy close
  replay rules, cents by their recorded policy.
- `shares.rs` `split_first` — the legacy first-absorbs split, cents.
- `tax_capital.rs` `RunningSplit::split` — the tax-basis K-1 worksheet's
  cents (after a `10`-place exact snap).

Display-only rounding in `ledger-gui`/`ledger-import` is K-U9's and was left
untouched. The figures there that reach an exported report are
`ledger-import/report.rs`'s `format_amount`/`amount_parts` (2 places,
half away) and `xlsx.rs`'s `exact_f64` (2 places); both are presentation, not
booked money.
