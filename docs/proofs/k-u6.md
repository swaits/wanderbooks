# K-U6: percentages and ratios (`kernel/ratio.rs`)

_Folded into `docs/verification.md` by the integrator; this note is the
unit's own record._

Kernel sweep unit U6 (`plan/kernel-sweep-audit.md` section 3). It serves
the contribution margin the projects page and the tools print
(BS-statement-5 statement 2, and wanderbooks' WB-statement-7), the
government's part of an operating payment (BS-statement-3 statement 8),
and the viewer's percentage columns (display only, untouched; see below).

## What remained after K-U3

K-U3 already routed the cash-flow government split's share through
`kernel::share_of` (`cashflow::split_part`). What was still outside the
kernel:

- `reports::contribution_margin_pct`: `contribution / revenue × 100`,
  rounded to one place half to even, `None` without revenue.
- `cashflow::split_part`: the exact shape around `share_of` — the rounded
  government part and the exact rest that sums to the payment. U3 had
  the share, not the named law that the two parts sum.
- Display-only percentages in `ledger-gui` (`pages::profit::pct`,
  `pages::reports`' change and share-of-income columns, `pages::capital`'s
  share rendering, `fmt::pct_change`'s chart percentage). These are class
  (b)/(c) in the audit: K-U9's display arithmetic, not kernel laws. They
  were left, with why, below.
- `shares::Percent` shares are already weights through
  `split_by_weights_with` (U3), and `tax::tax_summary`'s deductible
  percentages are already `kernel::deductible_amount`; neither is a new
  percent or ratio.

## The API

```rust
pub fn percent_of<T: Amount>(part: T, whole: T, step: T, mode: Rounding) -> Option<T>
pub fn split_ratio<T: Amount>(
    amount: T, a: T, b: T, step: T, mode: Rounding,
) -> Option<(T, T)>
```

`percent_of` is `part ÷ whole × 100`, division first (the order the
contribution-margin call site always used), rounded once with
`round_step`; `None` when the whole is zero or negative, the step is not
positive, or a quotient, product or result is not representable.

`split_ratio` is `amount × a ÷ (a + b)` (`share_of`) rounded once with
`round_step`, and the exact rest `amount − part`; it refuses a negative
share, a non-positive ratio, a non-positive step, or an unrepresentable
part or rest. The two parts sum to the amount by construction, exactly.

### What rounds, stated plainly (the K-U10 item)

`percent_of` divides first (`Decimal`'s own division: exact where it
terminates, otherwise rounded at 28 significant digits), then multiplies
by 100 (a 96-bit product that can round), then rounds once to the step in
exact integer arithmetic. `split_ratio` multiplies before dividing
(`share_of`: product first, then quotient), so `Decimal`'s product can
round first. For every figure the call sites produce (lines are capped at
10^15), both operations are exact and the result is the exact ratio
rounded once; at `Decimal`'s mantissa edge the reference and the engine
can differ. Both roundings are K-U10's to settle, not silent behaviour
changes here.

The `i64` model's division truncates *before* the percent's multiply, so
an arbitrary part could be a hundred points from the exact ratio. The Kani
percent harness therefore fixes the quotient exact (the part a whole
multiple of the whole); the bridge checks every fractional ratio on
`Decimal`, and Lean proves the exact-ratio law at any size.

## Kani harnesses (`src/proofs/ratio.rs`)

On `i64`, each with `kani::cover!`. The whole is fixed at 8 (both
functions divide by a whole or a ratio, and a symbolic divisor did not
finish in K-U2); the percent harness takes the part as `8 × k` so the
model's division is exact, and steps at 3 so a rounded percent moves. The
split is fixed at 1 : 2 and the amount runs −1,000 to 1,000.

| Harness | Law | Verification |
|---|---|---|
| `percent_of_within_half_step` | 44 | with the model's division exact (`part = 8 k`), the half modes put the percent within half a step of `100 k`; a percent not already a multiple of the step and a negative part are reached (2 covers). 7.4 s wall, 0.79 s verification |
| `percent_of_zero_whole_is_none` | 44 | a zero whole, a negative whole and a zero step are `None` for every part and mode; a nonzero part is reached (1 cover). 16.8 s wall, 0.13 s verification |
| `split_ratio_sums` | 44 | a 1 : 2 split's rounded part plus its exact rest is the amount, for positive and negative amounts; a part off the exact share and a negative amount are reached (2 covers). 16.5 s wall, 0.41 s verification |

Times: `cargo kani -p ledger-core --exact --harness proofs::ratio::<name>`,
the verifier's own verification time and the wall time including Kani's
build, the shared dev machine under `nice -n 15`, 2026-11-09.

The bookspec law each harness proves (machine-readable for the
traceability gate, `crates/ledger-core/tests/bookspec/traceability.rs`):

| Harness | Law |
|---|---|
| `percent_of_within_half_step` | 44 |
| `percent_of_zero_whole_is_none` | 44 |
| `split_ratio_sums` | 44 |

## Bridge (`tests/kernel_bridge/ratio.rs`)

- `i64_and_decimal_percent_agree_where_the_quotient_is_exact`: the Kani
  model in whole percent points and the engine in `Decimal`, where
  `part = whole × factor` divides exactly.
- `decimal_percent_matches_the_exact_reference`: an independent exact
  `i128` reference (integer numerator over denominator, rounded by mode,
  no kernel code and no `Decimal` division) for parts of every sign,
  wholes greater than zero, four scales and any step.
- `i64_and_decimal_split_agree_where_the_quotient_is_exact`: a 1 : 2
  split in cents against the engine in dollars where the amount is a
  multiple of three.
- `decimal_split_matches_the_exact_reference`: the first part is the
  exact share rounded once, at any amount, ratio and step.
- `decimal_split_sums`: on every input the parts sum to the amount, the
  rounded side is a whole multiple of the step, and a zero `a` share
  leaves the whole to the rest.
- `percent_of_refuses_no_whole`: a zero or negative whole, a zero ratio
  and a non-positive step are `None`, never a division by zero. A
  negative step refuses both functions.
- `decimal_ratio_at_the_edges`: `Decimal::MAX` (a part past the range
  refuses; a split either refuses or sums), scale 28 (a tiny part is
  zero, no negative zero; a ratio of one is 100), a whole past the range
  (a percent under a unit is zero), and a zero amount split whose two
  parts are asserted to be `+0` (a checked subtraction once handed the
  rest back as `-0`, which equality hides and a sign test does not). A
  zero part over a positive whole is likewise a `+0` percent.

All pass with the rest of `kernel_bridge`.

## Lean (bookspec `lean/Bookspec/Ratio.lean`, law 44)

`percent_near` (the percent is within half a step of the exact ratio,
`2 × |whole × percent − 100 × part| ≤ whole × step`, at any size),
`percent_zero`, `percent_sign`, `percent_neg`, `splitFirst_near` and
`splitRatio_sum`, with `whole_mul_percent` and `sum_mul_splitFirst` as
the exact-quotient lemmas. The model rounds `100 × part` to a multiple of
`whole × step` and divides by `whole` (exact, so the quotient is never
rounded twice), reusing `Round.near_halfEven`; it does not model
`Decimal`'s 28-digit division, which the bridge's exact reference carries.
No `sorry`; `#print axioms` shows only `propext`, `Quot.sound` and
`Classical.choice`.

BS-statement-5 links `percent_near`/`percent_zero`/`percent_sign` and
BS-statement-3 statement 8 links `splitRatio_sum`/`splitFirst_near`
(law 44 in `spec/laws.md`).

## Sites routed and left

Routed:

- `reports::contribution_margin_pct` → `kernel::percent_of` with step
  `0.1`, half to even; the display `rescale(1)` stays. The figures are
  unchanged (the kernel computes the same division-first product).
- `cashflow::split_part` → `kernel::split_ratio` with the currency's
  `10^-places` step, half to even; the shares' own sum still reports the
  old "party shares" refusal, and the split's errors the government-part
  one. The figures, including `−3.33/−6.67`, `0.02/0.03` and the reversal
  mirror, are unchanged.

Left, with why:

- `ledger-gui::pages::profit::pct`: it computes the same percent for a
  positive whole, but it also returns a percent for a *negative* whole
  (a refund-heavy P&L month), where `percent_of` is `None` by the margin
  rule. Routing it would change what is shown, so it stays display code
  (K-U9).
- `ledger-gui::pages::reports`' change/share-of-income columns: same
  reason (negative denominators are displayed today), and their rounding
  is display `round_dp(1)`, K-U9's.
- `ledger-gui::pages::capital::percent`: `share × 100` with
  `normalize()`, a stored ratio printed exactly, not `part / whole ×
  100`; no rounding is taken.
- `ledger-gui::fmt::pct_change`: a chart's percentage change in `f64`,
  not money and not a report figure (class (c)).
- `arap::percent_text`: basis points formatted as trade terms; integer
  text, not a ratio.
- `shares::Percent` and `tax::tax_summary`: already kernel arithmetic
  (U3's weights and `deductible_amount`).

## Gates

- `cargo test -p ledger-core`: all tests and every golden unchanged. It
  passes against the `k-u6` bookspec branch (patched checker/runner); the
  traceability gate alone fails against the still-pinned revision, because
  law 44 is not in it yet, exactly as every K-U law before the integrator
  moves the pin.
- `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`.
- Kani: the three harnesses above.
- The traceability gate against the `k-u6` bookspec branch (patched
  checker/runner), with `Cargo.lock` restored afterwards: 62 kernel
  functions (47 fully backed), 44 law rows, 133 claims sections, 1929
  backticked tokens, 404 source clauses; 15 exceptions, the pinned
  skipped set unchanged.
- bookspec: `lake build`, `lake env lean Audit.lean` (standard axioms),
  `cargo test --workspace`, `render -- --check`, `scripts/spec-floors.sh main`.
