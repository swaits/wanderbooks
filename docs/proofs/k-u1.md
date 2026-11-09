# K-U1: FX conversion, `fx_booked`

Status: landed 2026-11-08; review fixes and D-75 the same day. The kernel sweep's U1
(`plan/kernel-sweep-audit.md` section 3): the one foreign-currency
conversion that books money (`Ledger::add_line_fx`) moves into the
kernel, proved once in bookspec's Lean, checked on `i64` by Kani and
on `Decimal` by the bridge.

## The API

```rust
pub fn fx_booked<T: Amount>(foreign: T, rate: T, step: T, mode: Rounding) -> Option<T>
pub fn fx_booked_minor(foreign: Decimal, rate: Decimal, currency: Currency, mode: Rounding) -> Option<Decimal>
```

`fx_booked` is `foreign x rate` computed exactly (`checked_mul`),
rounded once to a whole multiple of `step` by `round_step` (never a
second rounding); `None` for a non-positive rate, a non-positive step,
or an unrepresentable product or result. The rate's precision is the
value's own: a `Decimal` rate keeps its scale (1.0834567 is exact), so
the product is the pre-kernel `checked_mul` product to the digit.
`fx_booked_minor` is the `Decimal`/currency spelling `add_line_fx`
books through: it picks `10^-currency.minor_units()` as the step, and
keeps the pre-kernel scale (an exact product at or under the minor
places unchanged, otherwise exactly the minor places; a currency with
no minor unit books the exact product, as `round_minor` always did).

**The precondition (D-75, owner, 2026-11-08).** `Decimal::checked_mul`
is the exact product only while the product keeps its full scale: the
two scales added within 28, and the two mantissas multiplied within
`Decimal`'s 96 bits (7.9e28). Outside that it rounds the product itself
(to 28 places, or to the places 96 bits hold), and `fx_booked` then
rounds the rounded value: a second rounding. The K-U1 review found it
with foreign `0.99999999999999999999999999` (26 places) x rate `0.005`
into USD: the exact product is 0.00499999999999999999999999995, under
half a cent, but `Decimal` holds 0.005 and the booking is 0.01.
`Ledger::add_line_fx` now refuses a foreign amount or a rate of more
than `MAX_LINE_SCALE` (10) decimal places (trailing zeros not counted),
with `LedgerError::ValueOutOfRange` ("foreign amount ..." / "FX rate
..."; the variant for an amount that is not a journal line's), before
computing and never on replay (`is_replaying()`; `replay_add_line_fx`
books a journal's older line verbatim, and a store whose journal holds
a 26-place foreign amount opens with its figures). Ten places each give
a product of at most twenty, under 28. `fx_booked`'s spec block states
the precondition, and states that `Decimal::checked_mul`'s two
behaviours outside it are inherited, not hidden: rounding past scale 28
(and past 96 bits), and `1e-28 x 1e-28` underflowing to zero.

**An open edge, not closed by D-75.** The 96-bit half of the
precondition is not enforced: ten places each still allow a product of
twenty places past about 7.9e8, which needs more than 96 bits.
`1050000010.0000000001 x 9.9999999999` is 10500000099.89499999999999999999
exactly (10500000099.89 to the cent) but `Decimal` holds
10500000099.895, and the engine books 10500000099.90. The bridge pins
this (`the_mantissa_edge_is_outside_the_precondition_and_pinned`) and
the spec block names it; refusing it is a decision for the owner (see
the report that came with this change).

Behaviour is unchanged for every input within the precondition (inputs
outside it are the ones above): `add_line_fx` keeps its own
refusals (`InvalidDocumentLineAmount` for a non-positive foreign
amount, `InvalidFxRate` for a non-positive rate or a result that
rounds to zero, `CurrencyMismatch`, `AmountOutOfRange` on overflow),
and every existing test and golden passed unchanged.

Other FX sites, by grep over `crates/ledger-core/src` and
`crates/ledger-store/src` for `fx_rate`/`rate`/`foreign`: the only
conversion is `add_line_fx`. `set_fx_rate`/`get_fx_rate` (core and
store) only record and return a rate for reference; the store's
`add_line_fx` already delegates to the core's; journal replay
(`replay_add_line_fx`) books a recorded amount verbatim and converts
nothing. None needed routing.

## Kani harnesses (`crates/ledger-core/src/proofs/fx.rs`)

All at `i64`, foreign symbolic in `[-256, 256]`, rate in `[1, 16]`,
over the fixed steps `1, 2, 5, 10, 100`. A symbolic step was not used:
K-U2 found CBMC's bit-blasted division by a symbolic `i64`
intractable (30 minutes unfinished), and `fx_booked` rounds through
`round_step`; the fixed steps include an even step (a tie exists),
odd steps, a decimal step, and steps above the products. The bridge
carries the same laws to every `Decimal` step, and Lean to any step.

| Harness | Law | Verification |
|---|---|---|
| `fx_booked_is_whole_unit` | the result is a whole multiple of the minor unit | 9.1 s (2 covers) |
| `fx_booked_within_half_unit` | within half a unit of the exact product | 16.4 s (3 covers: non-zero remainder, ties at steps 10 and 2) |
| `fx_booked_positive_for_positive` | positive inputs book non-negative, positive once the product reaches half a unit | 3.2 s (2 covers) |
| `fx_booked_refuses_overflow` | an overflowing product and a non-positive rate are `None`, never a panic | 2.6 s (2 covers) |

What the harnesses do and do not prove, stated plainly:

- `fx_booked_refuses_overflow` proves **two concrete overflow cases**
  (`i64::MAX x 2` and `100 x i64::MAX`, step 1) are `None`, plus that a
  non-positive rate is `None` for every symbolic rate <= 0. It is not a
  proof that every overflowing product is refused.
- That every overflow is `None` rests on `i64::checked_mul` (and
  `Decimal::checked_mul`) returning `None` when the product cannot be
  held, which the bridge exercises (`Decimal::MAX x 2`, the proptest's
  products), not on a Kani proof.
- **step <= 0 -> `None`** is not a harness of this unit: it is `round_step`'s
  refusal, proved by K-U2's `round_step_refuses_nonpositive_step`
  (`docs/proofs/k-u2.md`), and checked here by the bridge (`fx_booked(100, 1, 0)` is `None`).
- Every Kani law above is on the integer model, where the product is exact
  by construction (`foreign * rate` is a bounded `i64` product, no
  scale, no rounding before `round_step`). The D-75 precondition is
  what carries those laws to `Decimal`: with it, `Decimal::checked_mul` is
  the same exact integer product of mantissas, and the bridge compares
  the engine to an independent exact reference (below).

Measured alone on this machine, 2026-11-08, with
`cargo kani -p ledger-core --exact --harness proofs::fx::<name>`
(verification time; wall time includes the build). `just prove`
(fast set) passes with these four included, Lean at 151 theorems,
standard axioms only.

## Bridge (`crates/ledger-core/tests/kernel_bridge/fx.rs`)

- `i64_and_decimal_agree`: the Kani model and the engine agree on
  plain-integer conversions, value for value.
- `decimal_matches_the_exact_reference`: an independent `i128`
  reference (exact integer product, one integer rounding; no kernel
  code, no `Decimal` division) over foreign amounts and rates of every
  scale D-75 allows, 0 to 10 places each (products to scale 20), with
  mantissas to 2^47 (so the product fits 96 bits), booked into 0, 2,
  and 3 minor units (JPY, USD, KWD), through both `fx_booked` and
  `fx_booked_minor`. A result `Decimal` cannot hold is `None` in both.
- `ten_places_each_is_exact_through_the_engine`: the top of D-75's
  range through `Ledger::add_line_fx` and the reference, a hand-checked
  row each side of half a cent.
- `past_ten_places_the_reference_and_decimal_part_and_the_engine_refuses`:
  the D-75 bound is what keeps the reference and the engine equal. The
  review's 26-place input has reference 0.00 and kernel 0.01 (the
  kernel inherits `Decimal`'s early rounding); `add_line_fx` refuses it
  and any 11-place foreign amount or rate (`ValueOutOfRange`).
- `the_mantissa_edge_is_outside_the_precondition_and_pinned`: the open
  edge above, pinned as it is.
- `fx_booked_at_the_edges`: 100.00 x 1.0834567 to the cent, 10.00 x
  151.237 to the yen, a KWD half-thousandth tie, half-unit ties of
  both signs, scale 28 (including `Decimal`'s multiplication
  underflowing the finest product to zero, kept exactly as before),
  `Decimal::MAX` at rates one (books) and two (`None`, never a
  panic), a non-positive rate and step, the booked scale ("1.50",
  "108.3"), and gold's exact product.

All pass with the rest of `kernel_bridge`.

## Lean (bookspec, branch `k-u1`)

`lean/Bookspec/Fx.lean`, law 41 in `spec/laws.md`, headline theorems
in `lean/Audit.lean`. Build: `lake build && lake env lean Audit.lean`,
standard axioms only; `cargo test --workspace` and
`render -- --check` pass (checker law count 40 to 41).

- `Fx.booked_whole` — the booked amount is a multiple of the unit.
- `Fx.booked_near` — within half a unit of the exact product, any size.
- `Fx.booked_pos` — positive inputs book non-negative, and positive
  once the product reaches half a unit, the tie included
  (`step <= 2 * product`; first landed as `step < 2 * product`, short of
  the stated law and of the Kani assertion `2 * exact >= step`; the tie
  follows from `Round.tie_halfAway_pos` with `k = 0`).

`Fx.booked` is `Round.roundStep .halfAway` of the product, so the
three theorems restate `Round.roundStep_multiple`,
`Round.near_halfAway`, and `Round.sign_preserved` for the product
(the half-away mode is `Deductible.roundHA`; nothing reproved).
BS-ledger-1's FX invariants now name `Fx.booked_near`,
`Fx.booked_whole`, and `Fx.booked_pos` where the theorem matches;
its tie invariants keep `Deductible.roundHA_tie_*`.

BS-ledger-14 (the CHOICE split out of BS-ledger-1, S-C7) is on bookspec
`main`; its two FX invariants now cite `Fx.booked_near` (the exact-product
rounding) and `Fx.booked_whole` (every booked amount in `Z * u`). The
D-75 bound is stated in BS-entry-4 as an engine-neutral MUST (a foreign
amount and an exchange rate each take at most ten decimal places, trailing
zeros not counted; replay exempt, BS-entry-8), with D-75 as its
provenance. wanderbooks' claim for BS-entry-4 (`docs/spec/claims/entry.md`)
lists the enforcement (`add_line_fx`, `ValueOutOfRange`, never on replay)
and the checks.
