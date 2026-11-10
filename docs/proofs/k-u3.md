# K-U3: proportional shares and weighted splits

_Folded into `docs/verification.md`, section "Shares by weights
(BS-partner-1, BS-ledger-1)"._

Status: landed 2026-11-09. The kernel sweep's U3
(`plan/kernel-sweep-audit.md` section 3): the six local copies of
"share = whole × weight ÷ total, round, hand out the leftover" call one
implementation; the laws are proved on `i64` by Kani, checked on
`Decimal` against an independent exact reference, and stated once in
bookspec's Lean.

## The API

```rust
pub enum Leftover { LargestRemainder, LastAbsorbs, FirstAbsorbs }

pub fn share_of<T: Amount>(whole: T, weight: T, total: T) -> Option<T>

pub fn share_floor_with<T: Amount>(
    whole: T, weight: T, total: T, step: T,
    round: impl Fn(T) -> Option<T>,
) -> Option<(T, T)>

pub fn split_by_weights_into<T: Amount>(
    whole: T, weights: &[T], total: T, unit: T,
    round: impl Fn(T) -> Option<T>, policy: Leftover,
    parts: &mut [T], remainders: &mut [T],
) -> Option<()>
pub fn split_by_weights_with<T: Amount>(
    whole: T, weights: &[T], total: T, unit: T,
    round: impl Fn(T) -> Option<T>, policy: Leftover,
) -> Option<Vec<T>>
```

`share_of` is `whole × weight ÷ total` through the new
`Amount::checked_div`: `Decimal::checked_div` (exact where it terminates,
rounded at 28 significant digits otherwise) for the engine, truncating
integer division for a machine integer. The product is a
`Decimal::checked_mul`, which rounds at 96 bits too. That is the
computation every call site made before U3, to the digit.

`share_floor_with` is the exact floor of a share to a step, found by
multiplication rather than by trusting the quotient, plus the exact
remainder scaled by `total`. It is the arithmetic of the 1099 worksheet's
old correction loop, now bounded: past `MAX_CORRECTION_STEPS` steps in
either direction it returns `None` instead of walking without end.

`split_by_weights_into` is the split itself. The caller supplies the
total (not necessarily the sum of the weights: the conventions' shares
sum to one, the K-1 and party splits to their own total) and the
rounding function (the call sites pass `round_places`, whose scale rule
is what preserves the filed figures' scale; the Kani model passes
`round_step`). The leftover policy is explicit: largest remainder,
first absorbs, or last absorbs. The cumulative policy stays
`apportion_cumulative`, the running version, which `tax_capital` already
used. `split_by_weights_with` is the same split allocating its two
buffers; it and `share_floor_with` are the public API (`share_floor` and
`split_by_weights` had no engine caller and are gone).

`Amount::checked_div` is the one trait addition. It is documented as the
engine's division, not an exact-rational claim; `checked_div_exact` is
that one.

## Kani harnesses (`crates/ledger-core/src/proofs/allocate.rs`)

All at `i64`, fixed arrays (no symbolic-length `Vec`, no
`copy_from_slice`), each with `kani::cover!`. The divisor is fixed in
every harness: CBMC's bit-blasted division by a symbolic `i64` did not
finish in 30 minutes in K-U2, and every split divides by a total or a
step. The fixed weights are 2/3/5 of 10 (three different weights, a tied
split when the whole divides), the step is 1 (or 10 for the floor
harness), and the bounds are `whole` 0 to 1,000 (100 for the floor
harness). The laws are about the shape of the split, not the magnitude;
Lean proves them of the exact-share model at any size, and the bridge
checks the `Decimal` engine, whose ranking is the exact remainder (K-U10:
`whole × weight − part × total`, ties to the earlier part, the same
quantity `share_floor_with` returns).

| Harness | Law | Verification |
|---|---|---|
| `split_by_weights_sums` | 18 | the parts sum to the whole exactly; a shortfall is handed out; the largest weight takes more. 23.9 s (2 covers) |
| `split_by_weights_within_one_step` | 42 | each part is above `-total` and below `total` from its share, scaled by the total, and a part whose share is exact takes no unit; a part off its share, and a unit handed out while the first part's share is exact, are reached. 22 s (2 covers) |
| `split_by_weights_no_part_exceeds` | 18 | no part is negative or above the whole. 12.4 s (1 cover) |
| `split_by_weights_first_absorbs` | 18 | the first part is its rounded share plus the shortfall; the sum is the whole; no part exceeds it. 2.3 s (1 cover) |
| `split_by_weights_last_absorbs` | 18 | the last part is the whole less the others; the sum is the whole; no part exceeds it. 4.3 s (1 cover) |
| `split_by_weights_zero_total_is_zero` | 18 | a zero total is zeros; all-zero weights hand the whole out by largest remainder while the shortfall fits, and refuse past it. 4.2 s (1 cover) |
| `share_of_refuses_overflow` | 42 | `share_of(whole, 2, 1) = whole × 2` exactly when the product fits; a zero total and an overflowing product are `None`, never a panic. 0.47 s (2 covers) |
| `share_floor_is_the_exact_floor` | 42 | the part is a multiple of the step, one more step passes the share, and the remainder is exact below `step × total`; the correction down is reached. 5.1 s (2 covers) |

The upper bound of `split_by_weights_within_one_step` was inclusive on
the `i64` model until K-U10: `i64`'s division truncates, the ranking read
the quotient, and a part that divided exactly could take the unit another
part's truncation left over (`whole = 5`, weights 2/3/5: the first's 1
became 2). The ranking is now the exact remainder, which no division
touches, so the split is 1, 2, 2 and the bound is strict on `i64` as on
`Decimal`. Kani said so itself: with the exact ranking the old cover "a
part exactly one step above its truncated share" could not be reached,
and `just prove` fails on a cover that cannot. The harness now asserts
the strict bound and that an exact share takes no unit.

`share_floor_is_the_exact_floor` needed `#[kani::unwind(4)]`: without a
bound on the correction loops, CBMC tried to unwind them without end and
the harness ran past an hour at 1,000 × 100. With the bound and smaller
bounds (whole to 100, weight to 10, total 10, step 10) it verifies in
5.3 s. The bound is enough because half-away's first estimate is at most
one step above the exact floor and the increment loop at most one step
below; the bridge's exact reference carries the law for every other
step and magnitude. The loops also carry `MAX_CORRECTION_STEPS` now, so
a malformed input refuses; that bound only adds a condition inside the
same loops, and `unwind(4)` still holds.

Measured alone on this machine, 2026-11-09, with
`cargo kani -p ledger-core --exact --harness proofs::allocate::<name>`.

`just prove` (fast set): **80 harnesses**, 0 failures. The recipe's
1800-second batch cap was hit twice while the shared machine carried a
load average of 26-33 (the log's completed harnesses all passed, the
batch was killed while the rest ran); the same fast set run manually
with `-j 8` verified **80 of 80 harnesses, 0 failures** (the heaviest
harness 500.8 s, the run inside its 3600-second cap). Lean:
`verify/lean` 151 theorems, standard axioms only.

## Bridge (`crates/ledger-core/tests/kernel_bridge/allocate.rs`)

- `i64_and_decimal_agree`: the Kani model in cents and the engine in
  dollars agree value for value where the shares divide exactly (the
  `i64` quotient is then the same exact share, so the largest-remainder
  recipient is the same).
- `decimal_split_satisfies_the_kani_laws`: for every whole, remainder or
  not, the `Decimal` parts sum to the whole, each is within one step of
  its share (strictly, scaled by the total), and none exceeds the whole.
- `decimal_matches_the_exact_reference`: an independent `i128` reference
  (integer products and remainders, no kernel code and no `Decimal`)
  over weights 2/3/5 of 10, whole in cents.
- `decimal_matches_the_exact_reference_weights_sum_to_one`: the same
  reference with weights 0.5/0.3/0.2 and total one.
- `leftovers_at_each_policy`: first and last absorbs against their own
  reference; the parts still sum.
- `zero_weights_one_member_and_many`: zero weights (the handout and the
  refusal past one unit per part), one member, and one to ten equal
  members, against the reference.
- `share_floor_matches_the_exact_reference`: `share_floor_with` against
  the exact `i128` floor for non-terminating totals (3, 7, 11), any step
  to 100; the quotient's 28-digit rounding is corrected.
- `split_at_the_call_sites_scales`: the K-1 property split (401 by
  750/250), a cent 50/50, whole yen (101 by 51/51), and the exact
  no-rounding split, at the scales the call sites produce.
- `decimal_edges_are_none_or_exact`: `Decimal::MAX`, a product past it, a
  zero total, and scale 28: each `None` or exact, never a panic.
- `largest_remainder_ranks_like_the_exact_reference`: random weights (ties
  included, since K-U10) at whole units and at the cent, against the exact
  `i128` reference's ranking.
- `the_absorbing_parts_keep_the_old_scale`: the strings, scale included,
  for 4860 by 6/0.7/0.5 (first `4050`, as the old `split_first` gave) and
  902 by 1/0.000001/0.000001 (first `902.00`); last absorbs against the
  old `plug_last` and the no-places conventions split.
- `split_into_matches_the_allocating_split`: the fixed-array
  `split_by_weights_into` and `split_by_weights_with` agree, scale
  included (the gate requires a bridge that calls the fixed-array form).
- `ties_go_to_the_earlier_part`: 20.00 split 4/1/1 over 6 is
  13.34/3.33/3.33, and 20 whole is 14/3/3 (K-U10: the ranking is exact;
  the rounded quotient gave 13.33/3.34/3.33 and 13/4/3). A negative total
  keeps the share's own order.
- `share_floor_refuses_past_the_correction_cap`: the review's
  0.1113 × 5909471247797156928749568911.8 ÷ 0.11134381908633486049673
  returns `None` promptly instead of hanging.
- `finding_the_quotients_rounding_can_cross_a_step`: the K-U10 case
  below.

## Lean (bookspec, law 42)

`lean/Bookspec/Apportion.lean`, law 42 in `spec/laws.md`, headline
theorems in `lean/Audit.lean`. Build: `lake build && lake env lean
Audit.lean`, standard axioms only; `cargo test --workspace`, `render --
--check` and `scripts/spec-floors.sh main` pass.

- `Apportion.exact_shares_sum` — with the weights summing to `W`, the
  exact shares' numerators `x · w_i` sum to `x · W`; no weight is lost or
  invented.
- `Apportion.shares_of_near` — a split part is within one step of its
  exact share, gain or loss. It is `allocW_drift` under the name the
  kernel's split uses; nothing is reproved.

BS-partner-1's invariants now link `exact_shares_sum` and
`shares_of_near` (the split's own framing of the existing
`allocW_drift` bound). `Apportion.apportion_sum`, `apportion_drift` and
the `allocW_*` theorems were reused, not restated.

## Findings

1. **No behaviour change for posted values.** Every golden and every
   existing test passes unchanged. The six sites keep their own rounding
   mode, step, leftover policy, sign handling and error:
   - `tax_capital::RunningSplit::share` (used by `split` and `unsplit`)
     → `share_of`;
   - `k1::share_property` → `split_by_weights_with`, total = Σ credits,
     `10^-places`, toward zero, largest remainder (`shares::share_by`
     reaches it through this);
   - `conventions::split` → `split_by_weights_with`, total = one,
     `10^-places`, toward zero, largest remainder; no places → identity
     rounding, last absorbs;
   - `shares::split_first` → `split_by_weights_with`, total = Σ weights,
     the cent, toward zero, first absorbs;
   - `form1099::vendor_part` → `share_of` then `round_places` (the
     `payment == target` and `target == 0` short cuts stay);
     `form1099::split_share` → `share_floor_with` per bucket then
     `apportion` (the hand-rolled correction loop is gone);
   - `cashflow::split_part` → `share_of` then `round_places`.
   `periods::allocate` and `periods::allocate_last_absorbs` are frozen
   replay rules: their arithmetic is untouched, and their doc comments
   now name the kernel policy each equals. `k1.rs`'s test helper named
   `split` at ~684 is not arithmetic (it reads `k1_detail` rows) and
   needed no routing.
2. **The `i64` model's division truncates.** The Kani within-one-step
   bound was therefore inclusive on `i64` while the ranking read the
   quotient; since K-U10 ranks the exact remainder it is strict there
   too (above).
3. **K-U10, Decimal's edges.** At `Decimal`'s 96-bit mantissa edge,
   `checked_div`'s rounding can move a share across a step, and
   `checked_mul`'s rounding hides it from `share_floor_with`'s correction.
   Pinned by `finding_the_quotients_rounding_can_cross_a_step`: the exact
   share is just above the whole, the quotient rounds to one cent above
   it, the largest-remainder split refuses (a part above the whole is
   never emitted), and the floor keeps the rounded step. That pinned case
   is the one no posted line reaches (the line cap is 10^15). The
   product's rounding is reachable: a scale-10 line below the cap
   multiplied by itself, `123456789.0123456789²`, gives
   `15241578753238836.750190519988`, not the exact square. Both roundings
   are K-U10 items: it is for K-U10 to settle whether the kernel should
   refuse either outright.
4. **One panic became a refusal.** `k1::share_property`'s old
   `apportion(...).expect(...)` would panic at the edge above (a
   shortfall of one unit per part is not rounding); the kernel returns
   `None` and the site reports `TooLargeToCompute`. Unreachable with
   line-capped credits, and the bar is no panic.
5. **The correction is bounded.** The floor harness's loops needed
   `#[kani::unwind(4)]`, and the review found an input whose rounded
   product put the estimate far from the exact floor, so the old loop
   did not terminate: `0.1113 × 5909471247797156928749568911.8 ÷
   0.11134381908633486049673`. The kernel now takes at most
   `MAX_CORRECTION_STEPS` in each direction and returns `None` past the
   cap; no figure the loop produced within the cap changes. The bridge's
   `share_floor_refuses_past_the_correction_cap` pins the input, and its
   exact `i128` reference carries the same law for every step and
   magnitude.
6. **The first part keeps the old scale.** The old `shares::split_first`
   computed `parts[0] + (whole − Σ parts)`; the first kernel version
   computed `whole − Σ rest`. The values are equal, but the scale differs
   in 665 of 400k fuzzed cases: 4860 by 6/0.7/0.5 gave `4050, 472.50,
   337.50` and the kernel's `4050.00, …`; 902 by 1/0.000001/0.000001 gave
   `902.00` and the kernel's `902`. That scale reaches `Part.amount`, so a
   printed share moved. The kernel computes the old expression now (with
   checked arithmetic), and `the_absorbing_parts_keep_the_old_scale`
   compares strings. `LastAbsorbs` was already the old `plug_last` and the
   no-places conventions split, and the same property checks it.
7. **Ties go to the earlier part, exactly (K-U10).** The first version of
   `split_by_weights_into` (and the pre-U3 call sites) ranked the leftover
   units by the rounded quotients' remainders, so an exact tie went where
   the quotient's 28th digit sent it: 20.00 split 4/1/1 over 6 was
   `13.33, 3.34, 3.33` and 20 whole was `13, 4, 3`. It now ranks the exact
   integer remainder `whole × weight − part × total` (the quantity
   `share_floor_with` returns; the sign flipped for a negative total), so
   a tie goes to the earlier part, as `apportion`'s docs and bookspec's
   `Apportion.lean` (`remW`, `allocNat`: ties to the earlier member) say:
   `13.34, 3.33, 3.33` and `14, 3, 3`. Pinned by
   `ties_go_to_the_earlier_part`; `tests/kernel_bridge/ties.rs` keeps the
   old ranking as a reference and measures what the change moves. A
   recomputed past report can differ from what it showed before by a cent
   between parties (a posted entry never moves).
