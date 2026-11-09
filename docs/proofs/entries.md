# Entries: the party direction and a part's weight

Status: landed 2026-11-08 with the traceability gate's review fixes. Two
kernel functions that had no proof of their own now have one; both were on
the exception list before.

## The functions

| Function | Meaning |
|---|---|
| `kernel::Direction::of` | the direction of a signed amount: paid above zero, received below zero, neither at zero (BS-partner-3) |
| `kernel::part_weight` | a month part's weight in `MONTH_L`ths: `days × MONTH_L / month_days`, exactly; `None` for a part no calendar month has (BS-asset-1, BS-prepaid-1) |

## Kani harnesses (`crates/ledger-core/src/proofs/mod.rs`)

| Harness | Law | Bounds |
|---|---|---|
| `direction_of_is_the_sign` | 23 | any `i64` amount; all three signs covered |
| `part_weight_is_days_of_the_month` | 27 | `days` and `month_days` 0..=40; valid, partial, whole and invalid parts covered |

`part_weight_is_days_of_the_month` proves the weight formula a proration
charge is computed from, which is law 27's statement (each part but the
last is its days times the monthly rate); it is a lemma for laws 26-28,
not a restatement of law 26 (that the parts sum to the total, which is
`prorate_parts_sum_to_total` below).

The bookspec law each harness proves (machine-readable for the
traceability gate, `crates/ledger-core/tests/bookspec/traceability.rs`):

| Harness | Law |
|---|---|
| `direction_of_is_the_sign` | 23 |
| `part_weight_is_days_of_the_month` | 27 |

Times: measured alone with
`cargo kani -p ledger-core --exact --harness proofs::<name>` on 2026-11-08:
`direction_of_is_the_sign` 18 s wall (0.06 s verification, 3 covers, build
included); `part_weight_is_days_of_the_month` 11 s wall (0.49 s
verification, 3 covers, build included).

## Bridge (`crates/ledger-core/tests/kernel_bridge/entries.rs`)

- `direction_of_agrees`: `Direction::of` classifies the same sign at `i64`
  and `Decimal`, and each case keeps its meaning (law 23).
- `part_weight_agrees`: `part_weight` is the same weight at `i32`, `i64`
  and `i128`, and `None` exactly for a part no month has (laws 26-28).
- `reverse_target_agrees`: `reverse_target` is the exact opposite at both
  types. The edge strategy reaches `i64::MIN`, which `i64` refuses because
  no negation can hold it; `Decimal` can hold the negation, so the property
  compares its answer there (the exact opposite). The refusal is the
  integer proof model's bound, not the law's. The earlier bridge only
  passed it to `and_then`, which is a mention, not a call; this property
  calls it.

Why these laws: `Direction::of` is the sign classification the party
direction law (23) is stated over, and `part_weight` is the weight the
proration charges of laws 26-28 are computed from, its harness numbered
for the law it restates most closely (27).
