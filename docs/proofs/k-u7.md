# K-U7: payroll's net pay (`kernel/payroll.rs`)

_Folded into `docs/verification.md` by the integrator; this note is the
unit's own record._

Kernel sweep unit U7 (`plan/kernel-sweep-audit.md` section 3, U7). It
serves a payroll run's net pay and its entry plan: gross wages and the
employer FICA share debited, the withholdings and both FICA shares
credited to the 2110–2150 payables, net pay credited to cash (IRS Pub.
15, `docs/spec/sources/irs-payroll.md`: every withholding number is the
caller's; there is no tax table). The bookspec rules are BS-entry-3
(exact amounts), BS-entry-4 (every payroll figure, and every derived
figure, computed with checked arithmetic) and BS-entry-1 (debits equal
credits). Law 45 in bookspec's `spec/laws.md`.

## What remained after K-U4

K-U4 already took the collection totals: `payroll_entry_plan`'s sums
were checked folds, `remit_payroll_taxes` and the store's remittance
refuse through `money::sum_exact_of` before they plan, and the MCP
payroll totals go through `checked_sum`/`sum_exact_of`. What was still
outside the kernel:

- `PayrollLine::net_pay`: gross less the five employee withholdings, a
  hand fold of `Decimal::checked_sub` in `payroll.rs`.
- `payroll_entry_plan`'s per-column totals: seven local checked folds
  (gross, employer FICA, federal, state, Social Security, Medicare,
  other) plus the net total over each line's `net_pay`.
- `payroll_remit_plan`'s raw `total += *balance` (see "Fixed in review" below).

## The API

```rust
pub fn net_pay<T: Amount>(gross: T, deductions: &[T]) -> Option<T>
```

`net_pay` is gross less every deduction, folded left to right with
`checked_sub`: `Some` is the exact difference, and `None` the moment a
running difference cannot be represented — never a wrapped, rounded or
panicking result. The fold preserves the old `PayrollLine::net_pay`
behaviour exactly, including at the edge: `[MAX, MAX, -MAX]` from a
zero gross is `None`, where summing the deductions first would cancel;
each step must be exact, as `total`'s fold is. A net below zero is a
value, not a refusal: `validate_payroll_lines` refuses it as
`NegativeNetPay`, as before. `round_step` and `plug_last` have no
payroll use (nothing in a run rounds or splits) and were not called.

## Kani harnesses (`src/proofs/payroll.rs`)

On `i64`, fixed arrays, each with `kani::cover!`. A payroll line
carries five employee deductions, so the deduction arrays are fixed at
five; `net_pay`'s `None` half is proved on the whole `i64` range; the plan harness carries one employee and the plan's eight
totals in a fixed array of eight `(Side, amount)` lines. Amounts run to
a few thousand units (gross 1 to 10,000, withholdings to 1,000 or
2,000): the laws are about the shape of the subtraction and the
balance, every total stays far inside the `i64` line range (2^40) that
`check_lines` requires, and Lean proves the same laws at any size. The
plan harness takes the case where no plan total is zero (the real plan
skips a zero total; gross, employer FICA, both FICA payables and net
are positive by construction, and federal, state and other are assumed
positive).

| Harness | Law | Verification |
|---|---|---|
| `net_pay_is_gross_less_deductions` | 45 | net equals gross less the deductions' `total`, for deductions of either sign; withholdings exceeding gross and a negative net are reached, and `i64::MAX`/`MIN` past the range are `None` (2 covers). 19.5 s verification |
| `net_pay_none_iff_a_prefix_overflows` | 45 | gross and five deductions over the whole `i64` range: `None` exactly when some running difference (gross less the first k deductions, in `i128`) leaves the `i64` range, otherwise exactly the `i128` result; both branches reached (2 covers). 4.4 s verification (24.6 s wall). `i64::MIN` included: `Amount` for `i64` subtracts natively since K-U8 (the default negated first and refused `-1 - i64::MIN`, which fits; K-U7's and K-U8's harnesses both found it), and the harness asserts `net_pay(-1, [i64::MIN]) = Some(i64::MAX)` |
| `net_pay_nonnegative_when` | 45 | non-negative deductions summing to at most gross give exactly gross less their sum, never negative; a zero net and a partial withholding are reached (2 covers). 21.9 s wall, 7.0 s verification |
| `payroll_plan_balances` | 45 | one employee: the eight totals `payroll_entry_plan` posts pass `check_lines` (two or more lines, every amount positive and in range, debits equal credits); a net of exactly one unit, payables holding the employer shares alone (a zero employee Social Security and Medicare withholding) and a posted federal or state withholding are reached (3 covers, each able to fail under the assumes). 142 s wall, 135.5 s verification (2026-11-09, machine load average about 18; 62.0 s verification earlier, unloaded). The harness rebuilds the eight totals on `i64` rather than calling `payroll_entry_plan` (that works in `Decimal`/`Money` and maps accounts, neither generic over `Amount`): it proves the arithmetic the plan is made of and `check_lines` accepting that shape, while the assembly is checked by `payroll_entry_plan`'s tests, the goldens and `check_lines` on every posted entry. Two employees timed out in CBMC (over 80 minutes, killed): sixteen symbolic figures into eight totals. Kani covers one employee; Lean's `payroll_run_net_eq` and `payroll_run_plan_balances` carry any number of employees (see Lean below) |

Times: `cargo kani -p ledger-core --exact --harness
proofs::payroll::<name>`, the shared dev machine under `nice -n 15`,
2026-11-09. `payroll_plan_balances` is the slow harness of the unit:
`check_lines` sums the eight lines through `totals`, and two symbolic
employees make every column total a sum the solver must carry.

The bookspec law each harness proves (machine-readable for the
traceability gate, `crates/ledger-core/tests/bookspec/traceability.rs`):

| Harness | Law |
|---|---|
| `net_pay_is_gross_less_deductions` | 45 |
| `net_pay_none_iff_a_prefix_overflows` | 45 |
| `net_pay_nonnegative_when` | 45 |
| `payroll_plan_balances` | 45 |

## Bridge (`tests/kernel_bridge/payroll.rs`)

- `i64_and_decimal_net_pay_agree`: the Kani model in cents and the
  engine in dollars give the same net for the same gross and five
  deductions, of either sign.
- `decimal_net_pay_matches_the_exact_reference`: an independent exact
  `i128` reference (mantissas over a common power of ten, each running
  difference required representable at its own scale, no kernel code and
  no `Decimal` subtraction at the reference), compared by `to_string()`
  as well as by value (`Decimal` equality ignores scale) for a gross and
  up to five deductions of every sign and four scales.
- `decimal_net_pay_near_the_range_agrees_with_the_reference`: gross
  and up to five deductions within three units of `Decimal::MAX`/`MIN`,
  where the reference also returns `None`: the property meets overflow
  mid-fold (a prefix past the range is `None` even when a later
  deduction would bring it back), not only the edge table does.
- `decimal_net_pay_nonnegative_when`: deductions not exceeding gross
  give exactly gross less their sum, never negative.
- `decimal_net_pay_at_the_edges`: no deductions (the gross itself), the
  5,000.00 scenario (3,517.50), withholdings taking the whole gross
  (exactly zero), more than the gross (the exact negative difference),
  and past the range in either direction and mid-fold
  (`Decimal::MAX`/`MIN`, `[MAX, MAX]` from zero, and the same on
  `i64`): each is exact or `None`, never a panic.

All pass with the rest of `kernel_bridge`.

## Lean (bookspec `lean/Bookspec/Payroll.lean`, law 45)

`netPay` is the kernel's fold, `deductions.foldl (. - .) gross`, not a
closed form. The theorems:

- `foldl_sub_eq` and `payroll_net_eq`: the left-to-right fold is gross
  less the deductions' sum (induction).
- `payroll_net_nonneg`: not negative when the deductions do not exceed
  gross.
- `payroll_plan_balances` (the plan's `net` is zero) and
  `payroll_plan_debits_eq_credits` (stated directly through
  `Basic.net_eq_debits_sub_credits`), at the plan's scalar totals:
  accounts 0 to 7 carry Dr gross, Dr employer FICA, Cr the five
  payables (Social Security and Medicare with both shares) and Cr cash
  for the net.
- **Any number of employees**: `payroll_run_net_eq` (for a list of
  workers, each a gross and any number of deductions, the sum of each
  employee's net is the total gross less the total of every deduction)
  and `payroll_run_plan_balances` / `payroll_run_plan_debits_eq_credits`
  (the plan whose lines are per-employee sums and whose cash line is the
  sum of the per-employee nets, `runPlan`, balances for a list of any
  length). These carry the step `total_try(net_pay)` = gross less
  deductions that Kani does not (it takes one employee).

The model is total where the kernel is partial: `Int` does not overflow,
so the kernel's `None` half is carried by
`net_pay_none_iff_a_prefix_overflows` and the bridge. No `sorry`;
`#print axioms` shows only `propext`, `Classical.choice` and
`Quot.sound` (the standard trio).

BS-entry-3 links `payroll_net_eq`, `payroll_net_nonneg` and
`payroll_run_net_eq`, and BS-entry-1 links `payroll_plan_balances`,
`payroll_plan_debits_eq_credits`, `payroll_run_plan_balances` and
`payroll_run_plan_debits_eq_credits`, one theorem per invariant (law 45
in `spec/laws.md`).

## Sites routed and left

Routed:

- `PayrollLine::net_pay` → `kernel::net_pay` over the five employee
  withholdings; the `TooLargeToCompute` refusal is unchanged.
- `payroll_entry_plan` → `kernel::total_of` (gross, federal, state,
  other), `kernel::total_try` (employer FICA, both FICA payables, and
  the net over each line's kernel `net_pay`); the account mapping, the
  zero-skipping and the refusal are unchanged. The figures are
  unchanged: every golden and every test passes as it did.
- `payroll_remit_plan` (fixed in review): it did a raw `total +=
  *balance`, which panics on `Decimal` overflow, behind callers that
  pre-checked through `sum_exact_of` (an unenforced precondition on a
  `pub fn`). It now returns `Result<_, LedgerError>`, totals through
  `kernel::total_of` and refuses `TooLargeToCompute`; `Ledger` and the
  store propagate it and keep their own pre-checks.
  `remit_plan_past_exact_arithmetic_is_refused` covers it.

Left, with why:

- `Ledger::name_payroll_parties`' per-party `checked_add`: a grouped
  accumulation (each party's net over its lines), not a collection
  total; the single checked add is the kernel's `Amount` primitive and
  already refuses as `TooLargeToCompute`. No kernel shape fits it
  without restructuring the grouping.
- The MCP payroll totals and the store's remittance total: already
  kernel arithmetic through `money::sum_exact*` (K-U4).

## Gates

- `cargo test -p ledger-core`: all tests and every golden unchanged.
  `cargo test -p ledger-store` (payroll goes through the store).
- `cargo fmt --all --check`,
  `cargo clippy --workspace --all-targets -- -D warnings`.
- Kani: the four harnesses above.
- The traceability gate against the `k-u7` bookspec branch (patched
  checker/runner), with `Cargo.lock` restored afterwards: 63 kernel
  functions (48 fully backed), 45 law rows, 133 claims sections, 1962
  backticked tokens; 15 exceptions, the pinned skipped set unchanged.
  Against the still-pinned revision the same gate fails only on law
  45, which is not in it yet, exactly as every K-U law before the
  integrator moves the pin.
- bookspec: `lake build`, `lake env lean Audit.lean` (standard axioms
  only), `cargo test --workspace`, `render -- --check`,
  `scripts/spec-floors.sh main`.
