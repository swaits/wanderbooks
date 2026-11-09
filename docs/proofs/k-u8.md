# K-U8: control, aging and reconciliation totals (`kernel/control.rs`)

_Folded into `docs/verification.md` by the integrator; this note is the
unit's own record._

Kernel sweep unit U8 (`plan/kernel-sweep-audit.md` section 3). The audit
cites B-18, B-25 and B-38; the bookspec ids (`docs/spec/rules.md`) are
BS-arap-1 (aging), BS-arap-2 (a control account is its documents) and
BS-asset-4 (the section 704(c) built-in gain). The reconciliation itself
is BS-partner-4's (the book-to-tax capital identity), which the K-1
module already cites, so the functions cite it too.

## What remained after K-U4

K-U4 routed the plain sums through `kernel::total*`. Still computed by
hand, each a short chain at one call site:

- `arap::AgingRow::total`: `current + 30 + 60 + 90 + 90+`, a
  `try_fold(checked_add)`.
- `Ledger::control_check`: `difference = balance − documents_total −
  unattached`, two chained `checked_sub`s.
- `tax_capital::Reconciliation::total`: `beginning + built-in gain +
  property without a basis − property distributed`, three chained checked
  steps.

These are the identities the audit asked to name: the rules say the
buckets partition the open balances, the control difference is always
zero, and the reconciliation holds exactly. Naming them puts each under a
harness and a Lean theorem rather than leaving them as sums.

## The API

```rust
pub fn aging_total<T: Amount>(current: T, b30: T, b60: T, b90: T, b90p: T) -> Option<T>
pub fn control_difference<T: Amount>(balance: T, documents: T, unattached: T) -> Option<T>
pub fn reconciliation_total<T: Amount>(
    beginning: T, built_in_gain: T, without_basis: T, distributed: T,
) -> Option<T>
```

`aging_total` is `total([..five buckets..])`. The other two are the exact
chains `checked_sub, checked_sub` and `checked_add, checked_add,
checked_sub`, in the order the replaced code used, so a figure's scale and
a refusal are unchanged. All three return `None` rather than panic or
round.

## Kani harnesses (`src/proofs/control.rs`)

On `i64`, fixed arrays or plain bindings (no `Vec`, no `copy_from_slice`),
each with `kani::cover!`. No bound is fixed beyond the arity: each
harness quantifies over **arbitrary `i64`s** (the edge included) and
checks `Some(x)` against an exact `i128` specification, and (for
`aging_total_is_sum` and `reconciliation_total_identity`) that figures
within a journal line's range (2^40) always give `Some`.

| Harness | Law | Verification |
|---|---|---|
| `aging_total_is_sum` | 45 | five arbitrary `i64` buckets: `Some(t)` is the true sum; five in-range buckets always total; mixed signs reached (3 covers). 10.4 s verification, 25.6 s wall |
| `control_difference_identity` | 45 | three arbitrary `i64`s: `Some(d)` is `balance − documents − unattached`; `d = 0` exactly when `balance = documents + unattached`; agreeing in-range books differ by `Some(0)` (3 covers). 3.2 s verification, 30.9 s wall |
| `reconciliation_total_identity` | 45 | four arbitrary `i64`s: `Some(t)` is `beginning + gain + without − distributed`; in-range figures always total (1 cover; one `unreachable` check, which is the proof's `expect`, reported unreachable as it should be). 3.6 s verification, 39.6 s wall |

Times: `cargo kani -p ledger-core --exact --harness proofs::control::<name>`,
the verifier's own time and the wall time including Kani's build, the
shared dev machine under `nice -n 15`, 2026-11-09.

What the solver could not do: the **worksheet** statement, that a book
capital row and a tax capital row differing only by the three causes
differ at the ending by `reconciliation_total`. As a harness over nine
terms it did not finish in 500 s, and over seven (the common cash, income
and other terms cancelled) not in 110 s; the proof is the equality of two
differently associated 64-bit sums, which bit-blasting is poor at. It was
dropped, not weakened. Lean's `Control.reconciliation_identity` proves it
at any size, and `reconciliation_worksheet_identity` in the bridge checks
it on both `i64` and `Decimal`.

The bookspec law each harness proves (machine-readable for the
traceability gate, `crates/ledger-core/tests/bookspec/traceability.rs`):

| Harness | Law |
|---|---|
| `aging_total_is_sum` | 45 |
| `control_difference_identity` | 45 |
| `reconciliation_total_identity` | 45 |

## Bridge (`tests/kernel_bridge/control.rs`)

- `aging_total_agrees`, `control_difference_agrees`,
  `reconciliation_total_agrees`: `i64` cents against `Decimal`, and the
  plain `i64` sum, on random cents; agreeing books give `Some(0)`.
- `decimal_matches_i128_reference`: `Decimal` against an independent exact
  `i128` reference (running sum with signs, the 96-bit mantissa as the
  range) over the whole mantissa range with the edges `±(2^96 − 1)` and
  one in from them.
- `control_identity_edges`: `Decimal::MAX`/`MIN` in every position, each
  overflowing step `None`, each cancelling pair `Some(0)`; never a panic.
  (`0 − MIN` is `MAX`, representable: `Decimal` is symmetric.)
- `reconciliation_worksheet_identity`: the worksheet statement above, on
  `i64` and `Decimal`.
- `reconciliation_laptop`: the K-1 module's laptop scenario, −124.20.
- `kernel_matches_the_replaced_chains`: 20,000 random `Decimal`s of any
  scale (0 to 12) and mantissa up to 96 bits, the new function against
  the exact hand-written chain it replaced, compared as strings including
  the scale: identical for all three, `None`s included.
- `finding_decimal_add_rounds_scale_near_the_top`: see the finding.

## Finding (a K-U10 case, not a behaviour change)

K-U4's finding was that `rust_decimal`'s add rounds dust when a sum lands
on `Decimal::MAX`. The bridge's exact reference shows it is wider: any
add whose exact result needs more than 96 bits *at the operands' scale*
is not refused but **rounded to a coarser scale**. `0.01 +
792281625142643375935439503.35` (the largest scale-2 figure) returns
`792281625142643375935439503.4`, not `None`; a later step can bring the
scale back to 2, so a chain's result can differ from the exact figure at
scale 2 as well. The reference property therefore accepts a rounded
`Some` where the reference says `None` and pins one case as a canary, to
flip when `impl Amount for Decimal` is fixed. It is reachable only near
7.9 × 10^26 at scale 2 (a hundred billion times a journal line's cap), so
no ordinary path sees it, but it means "exact or `None`" is not yet true
of `Decimal` at its edge (K-U10's, with K-U4's finding).

## Lean (bookspec `lean/Bookspec/Control.lean`, law 45)

- `aging_partitions`: any documents placed in one of five buckets have
  five bucket sums that add (`agingTotal`) to the sum of their balances,
  by induction with `five_buckets_once`; `agingTotal_comm`.
- `control_difference_zero`: the control difference
  (`controlDifference`) is zero for any account, from
  `Documents.control_agrees`; `controlDifference_eq_zero_iff`.
- `reconciliation_identity`: a book row's ending less a tax row's ending
  is `reconciliationTotal` of the beginning difference and the three
  causes; `reconciliation_beginning_only`,
  `reconciliationTotal_causes`.

BS-arap-1's invariant `sum_b owed(b) = sum of open balances` and
BS-arap-2's aging invariant link `aging_partitions`; BS-arap-2 gains an
invariant for the control check's zero difference
(`control_difference_zero`); BS-partner-4's reconciliation invariant links
`reconciliation_identity`. Integers do not overflow in the model; the
kernel's `None` is the bridge's. The documents' day and the unattached
chain are `Documents`' own. No `sorry`; `#print axioms` shows only
`propext`, `Quot.sound` and (once) `Classical.choice`.

## Sites routed and left

Routed:

- `arap::AgingRow::total` → `kernel::aging_total`.
- `Ledger::control_check`'s difference → `kernel::control_difference`.
- `tax_capital::Reconciliation::total` → `kernel::reconciliation_total`.

The figures, scales and errors are unchanged: the replaced chains are
checked against the kernel as strings (`kernel_matches_the_replaced_chains`)
and every golden passes untouched.

Left, with why:

- `ledger-package::aging::aging_report`: the column totals use raw
  `+=` and `current + d30 + d60 + d90 + d90p`, which **panic** at the
  `Decimal` edge (an aging of documents restored "of any size"). The grand
  total is `aging_total` of the column sums; routing it would change a
  panic into a refusal, which is a behaviour change, and the `+=`
  accumulations need `kernel::total` first. A K-U10 case.
- `ledger-package::attention`'s overdue sum (`days_30 + 60 + 90 + 90+`)
  is already checked (`try_fold(checked_add)`) and is the overdue
  subtotal, not the aging total; left.
- `tax_capital`'s `book.beginning − beginning.known` (one checked
  subtraction) and the rows' `known` chain are single steps or the
  documented invariant 4 chain, already checked; no identity to name
  beyond what is routed.
- `ledger-gui::pages::reports`' aging totals: per-column checked sums
  already; display.

## Gates

- `cargo test -p ledger-core`: all tests and every golden unchanged,
  against the `k-u8` bookspec branch (patched checker/runner).
- `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`.
- Kani: the three harnesses above.
- bookspec: `lake build`, `lake env lean Audit.lean` (standard axioms),
  `cargo test --workspace`, `render -- --check`, `scripts/spec-floors.sh main`.
