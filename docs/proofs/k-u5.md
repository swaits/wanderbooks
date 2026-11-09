# K-U5: equal monthly parts (`kernel/schedule.rs`) and the versioned rounding rule

_Folded into `docs/verification.md`, section "Equal monthly parts
(BS-prepaid-1, BS-prepaid-2, BS-prepaid-4, BS-ledger-1)"._

Kernel sweep unit U5 (`plan/kernel-sweep-audit.md` section 3), with the
owner's decision D-72 (2026-11-08, "Version the rule"): **new monthly-even
schedules round to the currency's minor unit (it was two places for every
currency, wrong for the yen and the dinar); schedules written before the
format feature replay the cent rule, so nothing journaled moves.** The
authorities are bookspec `BS-prepaid-1` (a schedule recognizes exactly its
total over its months), `BS-prepaid-2` (equal months), `BS-prepaid-4` (the
choice D-72 picked), `BS-ledger-1` (whole smallest units) and `BS-entry-8`
(what is journaled never moves).

## The function

```rust
pub fn even_parts<T: Amount>(total: T, step: T, parts: &mut [T]) -> Option<()>
```

Fills `parts` (one slot per month) with `months = parts.len()` amounts that
sum to `total` exactly: every part but the last is `total / months`
rounded half to even to `step` (`round_step`), the last is `total` less the
others (`plug_last`). `None` for no months, a step that is not positive, or
an amount the type cannot hold; never a panic. Nothing is multiplied; the
only rounding is `round_step`'s exact integer arithmetic and the type's
own division.

The audit sketched `(total, months, step) -> Option<Vec<T>>`. The slice
form keeps `months` implicit, allocates nothing in the kernel (so there is
no allocating wrapper to except from the gate, as `movements` and
`reversal` needed), and lets the Kani harnesses use fixed arrays. The
caller (`schedules::plan_in_places`) allocates after it has checked the
month ends, so a plan that runs past chrono's last day is refused before a
part is made.

### What rounds, stated plainly

`even_parts` divides with `Amount::checked_div`: for `Decimal`, exact where
the quotient terminates and otherwise rounded at 28 significant digits;
for `i64`, truncating. It then rounds that quotient to the step exactly.
So for a total whose quotient fits 28 digits to well below the step (every
schedule: totals under `EXACT_LIMIT`, 10^26, and in practice a line limit
of 10^15), the part is the exact rational rounded once, half to even, and
the bridge's independent `i128` reference agrees. Near `Decimal::MAX` the
quotient carries a half-unit of its own: `MAX / 2 = ...167.5` has 29
digits, `Decimal` rounds it to `...168` first, and the part is not within
half a step of the exact quotient (`the_edges` pins it). The parts still
sum to the total, because the plug is exact. This is the K-U10 item (the
division's rounding); the engine's callers refuse totals of `EXACT_LIMIT`
or more, so no schedule reaches it. A single month is the whole total even
when finer than the step (the plug takes it, as the cent rule always did).

## How a schedule knows its rule (the D-72 decision)

**A flag journaled on the schedule, never the feature's sequence
position.** `AmortizationSchedule.rounding` is a `ScheduleRounding`
(`Cent` or `MinorUnit`); a monthly-even schedule written under
`MinorUnit` journals `"rounding": "minor_unit"` on its
`schedule.registered` line. A line with no `rounding` is the cent rule, as
every schedule before D-72 is, and replays exactly as it was posted.

Why not the position: a position (the schedule's `seq` against the
`journal.format_set` that declared the feature) is not in the core's
`Ledger`, would have to be threaded through every replay, snapshot and
adoption copy, and breaks the moment a replay starts mid-journal or a
schedule is carried to another set of books. A flag is the schedule's own
fact and survives all of those. The cost is one optional key, left unsaid
for the cent rule so a serialized ledger from before D-72 reads back, and
a cent schedule serializes, byte for byte as before.

The store chooses the rule when a schedule is registered (`Store::
register_schedule`): minor unit exactly when the books declared
`schedule_minor_unit` and the schedule is monthly-even; otherwise the cent
rule, so books waiting for the owner's format upgrade keep working as they
do and refuse a yen month finer than a yen as before. `ensure_feature`
guards the journaling of the flag. The core's plain
`Ledger::register_schedule` stays the cent rule (the core cannot know the
format, and its existing tests rest on the cent rule); the store calls the
new `register_schedule_under`.

**Older binaries refuse, never misread (BS-format-1).** The feature
`schedule_minor_unit` is in `KNOWN_FEATURES`, declared with every other
feature by new books and by an upgrade, before any `schedule.registered`
can carry the flag. A version that doesn't know it stops at the
`journal.format_set` as newer books
(`format::tests::schedule_minor_unit_is_its_own_feature`); a
`schedule.registered` naming a rounding this version doesn't know is
`NewerJournal` too, never corrupt
(`an_unknown_rounding_is_newer_not_corrupt`).

**A USD schedule gives the same plan under both rules: by construction,
and tested.** `ScheduleRounding::places` is two for `Cent` and the
currency's ISO 4217 minor unit for `MinorUnit` (two for USD); both rules
call the same `plan_in_places(total, places, ..)`, so for any currency with
two places the call is literally the same. The test
`the_cent_rule_is_unchanged_and_usd_is_the_same_under_both_rules` also
checks it over 300,000 plans, values and printed text.

### Replay fidelity: the printed scale

A posted month's replay compares its journaled lines as text, so the cent
rule must give the same figures at the same printed scale as before, not
only equal values. `round_places` (the old call) returns `0.250` as
`0.25` and a zero as `0.00`; `round_step` (the kernel's) returns an amount
already a whole multiple of the step as it came, and a zero as `0`. The
exhaustive test found it (`1 over 360` printed `1.00` against `1`): the
plan applies `present_as_rounded`, which restores the old scale (and the
last month's: `total` less the others at the wider scale, or the total's
own when the others are zero). Values never change; every rescale is to
or from trailing zeros, checked by debug assertions. The oracle is a
verbatim copy of the pre-K-U5 function in `tests/schedule_minor_unit.rs`,
compared over 300,000+ (total, scale, months) plans and a proptest up to
10^25, value and text.

## Kani harnesses (`src/proofs/schedule.rs`)

On `i64`, fixed arrays, each with `kani::cover!`. Totals -1,000 to 1,000;
months fixed to {1, 2, 3, 7} and steps to {1, 2, 5, 10, 100}. Symbolic
months or step were not used: `even_parts` divides by the months and
`round_step` by the step, and CBMC's bit-blasted division by a symbolic
`i64` did not finish in K-U2. Months {2, 3, 7} cover a tie, a quotient that
never ends and a divisor that divides few totals; the bridge carries every
month count, step and `Decimal` total, and Lean any number of months.

| Harness | Law | Verification |
|---|---|---|
| `even_parts_sum` | 43 | the parts sum to the total for every month count and step; the last takes a unit the others left (3 covers). 47.1 s |
| `even_parts_all_but_last_equal` | 43 | all but the last are equal, are the half-even rounded quotient, and a whole multiple of the step; a tie at step 10 is reached (2 covers). 23.9 s |
| `even_parts_last_absorbs` | 43 | the last is the total less the others, the first part plus what the others left; a single month is the whole total (1 cover). 31.4 s |
| `even_parts_within_half_step` | 43 | the first part is within half a step of the integer quotient, and `months x each` within half a step a month of the total plus the truncation, under a unit a month; an exact tie at half a step is reached (2 covers). 42.7 s |
| `even_parts_refuses_no_months_and_a_bad_step` | 43 | no months, a zero step and a negative step are `None` (1 cover). 0.27 s |

Times: `cargo kani -p ledger-core --exact --harness proofs::schedule::<name>`,
the verifier's own "Verification Time", the shared dev machine under
`nice -n 15` and `ulimit -v 12000000`, 2026-11-09. The `i64` quotient
truncates, so the bound against the true quotient is one unit a month
wider than half a step (`3 * step + 4` for three months); the `Decimal`
quotient keeps its fraction, and the bridge checks the strict bound.

## Bridge (`tests/kernel_bridge/schedule.rs`)

- `i64_and_decimal_agree_where_the_quotient_is_exact`: the Kani model in
  cents and the engine in dollars agree, value for value, at every step
  where the integer quotient is exact (1 to 600 months).
- `the_i64_laws_hold_for_any_month_count`: sum, all-but-last equal, whole
  multiple, within half a step of the quotient, last absorbs, for 2 to 600
  months and six steps. `one_month_is_the_whole_total`: one month, both
  types.
- `decimal_matches_the_exact_reference`: `Decimal`'s parts equal an
  independent exact reference (an `i128` numerator over an `i128`
  denominator, half to even, no kernel code and no `Decimal` division) for
  totals of up to 15 digits at every scale 0 to 6, either sign, 2 to 600
  months, steps 1, 0.01, 0.001, 0.0001 (JPY, USD, KWD, CLF).
- `the_decimal_laws_hold_for_any_month_count`: sum, equal, whole multiple,
  `2 |months x each - total| <= months x step`, and the last, on `Decimal`.
- `the_edges`: no months and a bad step are `None`; the three plans by hand
  (¥33,333 x 2 and ¥33,334; 333.33, 333.33, 333.34; 0.333, 0.333, 0.334);
  ties of both signs (5 over 2 is 2, 3; 7 over 2 is 4, 3; -7 over 2 is
  -4, -3); 0.01 over 3 at the cent (0, 0, 0.01); scale 28 (1e-28 over 2);
  `Decimal::MAX` and `MIN` at every step and month count (**not `None`: the
  parts exist and sum exactly, because the plug is exact; they never panic,
  and the half-unit the quotient carries is pinned, see above**).

Engine tests (`tests/schedule_minor_unit.rs` in ledger-core and ledger-store):
the hand-computed JPY and KWD plans, the cent-rule oracle, the USD
equivalence, the replay of a pre-feature schedule after the upgrade, an
unknown rounding is newer, a daily schedule carries no flag.

## Lean (bookspec `lean/Bookspec/Schedule.lean`, law 43)

`evenParts_sum` (reusing `plugLast_total`), `evenParts_length`,
`evenParts_all_but_last_equal`, `evenParts_last_absorbs` (the closed
form), `each_multiple`, `each_near` (reusing `Round.near_halfEven`),
`last_near`, with `n_mul_each` and `plugLast_replicate` as lemmas. The
model counts in whole steps and rounds the exact quotient once
(`roundStep .halfEven (n * step) whole / n`), so it does not model
`Decimal`'s 28-digit division; the bridge's reference carries that half.

## What is not covered

- The engine's two-place rule for a currency ISO 4217 gives no minor unit
  (gold, SDRs) is the daily plan's, not a law of this function.
- `Decimal`'s division at the 28-digit edge (K-U10).
- The pin: wanderbooks still pins the bookspec revision before this law;
  the integrator moves the pin (`verify/lean/lakefile.toml`,
  `lake-manifest.json`, the crates' `Cargo.toml`) after bookspec's
  `k-u5` branch is merged.
