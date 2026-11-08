# K-U1: FX conversion, `fx_booked`

Status: landed 2026-11-08. The kernel sweep's U1
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

Behaviour is identical for every input: `add_line_fx` keeps its own
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
  code, no `Decimal` division) over foreign amounts and rates of up
  to 10 places, booked into 0, 2, and 3 minor units (JPY, USD, KWD),
  through both `fx_booked` and `fx_booked_minor`.
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
  once the product reaches half a unit.

`Fx.booked` is `Round.roundStep .halfAway` of the product, so the
three theorems restate `Round.roundStep_multiple`,
`Round.near_halfAway`, and `Round.sign_preserved` for the product
(the half-away mode is `Deductible.roundHA`; nothing reproved).
BS-ledger-1's FX invariants now name `Fx.booked_near`,
`Fx.booked_whole`, and `Fx.booked_pos` where the theorem matches;
its tie invariants keep `Deductible.roundHA_tie_*`.

BS-ledger-14 (the CHOICE split out of BS-ledger-1) is absent from
bookspec `main` (`c467093`, the commit `k-u1` branches from). Its
record is located at `rules/ledger/BS-ledger-14.toml` in commit
`233a569` on the unmerged branch `s-c7a` (worktree
`~/Code/bookspec-wt/s-c7a`, clean at inspection). Linking it from
`k-u1` therefore needs an owner/integrator decision:

1. **(Recommended)** Land S-C7a (or the merged `s-c7`) on `main`
   first, rebase `k-u1`, then set BS-ledger-14's two FX invariants
   to `lean = "Fx.booked_near"` (the exact-product invariant) and
   `lean = "Fx.booked_whole"` (every booked amount in `Z * u`),
   regenerating the pages. No other record changes.
2. Cherry-pick `233a569` into `k-u1` now and link there. This was
   not done: that commit also creates BS-entry-6..8,
   BS-ledger-15..21, BS-period-4/5, BS-statement-6..9 and
   BS-prepaid-2..4, well outside this unit, and would collide with
   S-C7 at merge.

Outcome recorded here: option 1; the BS-ledger-14 link is the one
item left for the integrator, and everything it needs (theorem
names, which invariant each matches) is stated above.
