# K-U4: collection totals (`kernel/total.rs`)

Kernel sweep unit U4 (`plan/kernel-sweep-audit.md` sections 2 and 3). The
authority is BS-entry-3 (amounts are exact; rounding only on returns) and
BS-entry-4 (no input may reach a panic), bookspec `BS-entry-3` and `BS-entry-4`.

## The functions

| Function | Meaning |
|---|---|
| `kernel::total(values)` | exact sum of any figures; `None` the moment a running sum is not representable |
| `kernel::total_of(items, f)` | `total` of `f(item)`: the shape of `iter().map(f).sum()` |
| `kernel::total_try(items, f)` | like `total_of` for a mapping that can itself fail (`c - d`) |

The signed sum of `(Side, amount)` lines is **not** new: it is
`kernel::net` (and `kernel::totals` for the two columns), which stay the one
copy of that shape. The audit's proposed `signed_total` is exactly `net`;
none was added. The engine-side wrappers (`money::sum_exact`,
`sum_exact_of`, `sum_exact_try`, `add_exact`, `sub_exact`) only map `None`
to `LedgerError::TooLargeToCompute`; they call the kernel and add no
arithmetic.

## Kani harnesses (`src/proofs/total.rs`)

All on `i64`, fixed arrays (no symbolic-length `Vec`, no `copy_from_slice`,
model-checking/kani#4904), each with `kani::cover!`.

| Harness | Law | Bounds | Time |
|---|---|---|---|
| `total_empty_is_zero` | `total([])`, `total_of([])`, `total_try([])` are `Some(0)` | none | 0.1 s |
| `total_append` | `total(a ++ b) = total(a) + total(b)`, all `Some`, for in-range runs | two runs of 0..=3 amounts, each at most 2^40 | 60 s |
| `total_is_sound` | `Some(s)` means `s` is the true sum, for **any** `i64` figures (edge included) | 0..=4 arbitrary `i64` | 1.9 s |
| `total_of_is_map_sum` | `total_of` with the signed amount as the map is the signed sum, and equals `net` | 0..=4 lines, in-range | 7.4 s |
| `total_try_is_map_sum` | `total_try` of differences is the sum of the differences; a failing map fails the total | 0..=4 pairs, in-range | 5.3 s |
| `total_overflow_is_none` | for two arbitrary `i64`s, `total` is `Some` exactly when the true sum fits; both overflow directions covered | all `i64` pairs | 0.5 s |

The bookspec law each harness proves (machine-readable for the
traceability gate, `crates/ledger-core/tests/bookspec/traceability.rs`):

| Harness | Law |
|---|---|
| `total_empty_is_zero` | 4 |
| `total_append` | 4 |
| `total_is_sound` | 4 |
| `total_of_is_map_sum` | 4 |
| `total_try_is_map_sum` | 4 |
| `total_overflow_is_none` | 4 |

Times: alone, `cargo kani --exact --harness`, the shared dev machine under
`nice -n 10`, 2026-11-08. `total_append` was 115 s at runs of up to 4 and
was reduced to up to 3 (the same law; 60 s) to keep `just prove` fast. A
version with one array split at a symbolic point (`split_at`) did not
finish in 10 minutes and was dropped.

Why the bounds are enough: the law is about appending, so zero, one and
several figures on each side exercise every shape; `total_is_sound` covers
the overflow edge with unconstrained `i64`s. Beyond the bounds the Lean
statement carries it for any length (below).

## Order dependence at the edge

`total` is a left-to-right checked fold: every running sum must be
representable. So `total([MAX, 1, -1])` is `None` while
`total([MAX, -1, 1])` is `Some(MAX)`. `total_append` is stated for runs that
never leave the range (always true for line-capped amounts); `total_is_sound`
is the unconditional statement (a `Some` is never wrong). The bridge edge
table pins both orders.

## Lean

No new law (the audit said so; confirmed against
`~/Code/bookspec/lean/Bookspec/Basic.lean`, which `verify/lean`'s `Basic`
mirrors):

- `Basic.net_append` (`net (xs ++ ys) = net xs + net ys`) is the direct
  counterpart of `total_append` for the signed sum, i.e. `total_of` with the
  line's signed amount; `net_cons`/`net_nil` give the empty case.
- `Basic.sumOver_add`, `sumOver_neg`, `sumOver_zero` cover sums of a mapped
  field. There is no `sumOver_append` over index lists; for a plain list of
  integers the statement is core Lean's `List.sum_append`. If an explicit
  named theorem is wanted it is a three-line induction like `net_append`;
  I did not write Lean in this unit.
- Overflow (`None`) has no Lean counterpart: Lean's `Int` is unbounded.
  That half is carried by `total_overflow_is_none`, `total_is_sound` and the
  bridge edge table.

## Bridge (`tests/kernel_bridge/total.rs`)

- `total_agrees`, `total_of_agrees`, `total_try_agrees`,
  `total_append_agrees`: i64 vs `Decimal` on random runs (2,000 cases each).
- `decimal_edge_table`: empty, `Decimal::MAX`/`MIN` alone, one unit past
  either edge (`None`), `MAX + MAX`, `MAX + MIN` and `MAX + -MAX` cancel
  exactly, mixed signs cancelling in the middle, the order-dependent edge,
  `total_of`/`total_try` at the edge.
- `finding_decimal_add_rounds_dust_at_the_edge`: see finding 1.

## Findings

1. **`Amount::checked_add` for `Decimal` is not exact within one unit of
   `Decimal::MAX`.** `Decimal::MAX.checked_add(0.4)` is `Some(MAX)`:
   `rust_decimal` rounds sub-unit dust when the sum would need a 97th bit
   (`MAX + 0.5` is `None`). The kernel's "exact or `None`" contract
   therefore fails for figures in the top unit of `Decimal`'s range
   (~7.9 x 10^28, 10^13 times a line's cap), for every kernel function that
   adds, not only `total`. Not reachable by posted lines (capped at 10^15).
   Not fixed here: it lives in `impl Amount for Decimal`
   (`kernel/entries.rs`), shared by every unit. A canary test pins the
   present behaviour. Suggested fix: refuse any `Decimal` sum whose result
   scale is below the larger operand scale (rounding happened).
2. **The audit's section 2.1 overstated reachability.** `money.rs`
   `checked_add`/`checked_sub` were already checked;
   `ledger-gui/books.rs` `share_of_what_for` was already checked. A posted
   entry's lines are validated against the line cap when it posts, restored
   or new, so sums over posted lines (the reports) cannot reach the edge;
   the store's `create_invoice`/`create_bill` raw `.sum()` ran after
   `validate_lines` (cap). What genuinely panicked with a public caller's
   arguments: `audit::check_articulation` (`current_earnings + closed`),
   `periods::allocate_last_absorbs` / `allocate` (`amount * share`), and the
   kernel-less helpers with `Decimal` arguments. All now refuse. Documents,
   drafts and recurring entries are the figures BS-entry-4 lets restore oversized.

## Sites replaced

(file:line at this commit; each `sum_exact*` / `add_exact` / `sub_exact` /
`kernel::total*` use)

- **ledger-core** (about 90 sites):
  `reports.rs` 271-272 (P&L by class), 315-324 (balance sheet), 416
  (statement lines), 521-522 (P&L statement totals), 824-834 (balance sheet
  as of), 961-1020 (general ledger begin and running balances and column
  totals), 1200 (members' shares), 1235-1427 (members' equity: capital,
  flows, distributions, income, totals), 1797-1798 (asset register);
  `k1.rs` 183, 207, 227, 249, 277, 293, 304, 310; `tax_capital.rs` 591,
  823, and 759/`rest` (checked `-`); `form1099.rs` 707 (`sum3` was a copy of
  the kernel's sum, now `kernel::total`) and the `split_share` loop;
  `payouts.rs` 169-189; `audit.rs` 528 (and `closed_net`,
  `check_articulation`); `ledger.rs` 3824-3853 (close splits), 4061
  (member shares), 6414, 8434; `periods.rs` 127-180 (`try_allocate`,
  `try_allocate_last_absorbs`, the infallible rules delegate to them);
  `arap.rs` 1066 (`document_total` is now `kernel::total_of`).
- **ledger-store** `store.rs` 2570, 2575, 3450, 3745, 4454, 4501 (6).
- **ledger-package** `package.rs` 295, 297, 301, 318, 533, 542;
  `vendor_1099.rs` 196, 201; `text.rs` 35 (`total_text`) (8).
- **ledger-mcp** `server.rs` 1352; `tools/assets.rs` 843, 927;
  `tools/payroll.rs` 117; `tools/reports.rs` 35 (5).
- **ledger-gui** `pages/cash.rs` 186 (1).
- **ledger-import**: none (see below).

## Not replaced (infallible public signatures)

Also left raw, found by the review (2026-11-08), each bounded by the
posted-line cap or a validated input: `ledger.rs` ~2305 (a document's
mirror moves), ~4000 (`member_close_run`), ~8106 (`reconciliation_report`,
a fallible function: a candidate for unit K-U8, control and
reconciliation totals); `ledger-config` `lib.rs` ~362 (the members'
shares, each validated to (0, 1]); `ledger-import` `xlsx.rs` ~390; the
viewer's `pages/invoices.rs` ~203, `pages/reconciliation.rs` ~171,
`pages/reports.rs` ~468, `books.rs` ~142 (display, unit K-U9).

The regression test comparing `try_allocate` with `allocate` checks the
delegation only (`allocate` calls it). The review compared both frozen
rules, reimplemented from before this unit, over 20,000 random cases
and the edges: identical. A property test with that independent
reference is worth adding with unit V-D2.

A raw sum remains where the function returns a bare figure and changing it
to `Result` would change many callers and existing tests. Each is unreachable
with posted-line-capped figures; each is a signature decision, not a
refactor:

- `reports.rs` `trial_balance*` (movements, column totals), `project_entries`,
  `ProfitAndLossStatement::net_income` and `BalanceSheet::balanced`.
- `tax.rs` `TaxSummary::book_net`/`tax_net` (`sum` helper).
- `cashflow.rs` `kinds()`; `ledger.rs` `Regime` (`closed`, `given`);
  `payroll.rs` `payroll_remit_plan` (its caller now refuses first);
  `analysis.rs`, `conventions.rs`, `shares.rs`, `parties.rs` running sums.
- `ledger-import/report.rs` (`statement` builders return a bare `Statement`),
  `templates.rs` `equal_shares` (n owners, tiny).
- `ledger-gui/books.rs` view-model sums (display only).
- `tax_capital.rs` 485-502: multiplication by +/-1 and a value less its
  own truncation cannot overflow.
- Rounding calls: left for K-U2.

## Regression tests

- `ledger-core/tests/exact_totals.rs`: the helpers refuse at `Decimal::MAX`;
  `document_total`; `check_articulation(MAX, MAX, MAX)` (panicked before);
  `try_allocate*` refuse where `allocate*` panicked and equal them for
  ordinary input; ten lines at the cap still sum.
- `ledger-store/tests/exact_totals.rs`: an invoice and a bill of
  `Decimal::MAX` lines are refused and journal nothing.
- No test for the mcp previews or the gui cash page: their inputs are posted
  balances, which the line cap keeps far from the edge (finding 2), and the
  public tool API cannot construct an oversized one.

## `just prove`

`nice -n 10 just prove` on the shared dev machine, 2026-11-08: **59 Kani
harnesses verified** (53 before, plus these 6; every cover satisfied), then
Lean (118 headline theorems, standard axioms only); 616 s wall clock for
the whole recipe under load from other workers' builds (Kani's own
reported time for the heaviest harness of the run was 235 s).
