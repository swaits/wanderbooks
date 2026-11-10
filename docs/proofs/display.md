# K-U9: display rounding, and percentages in Decimal

_Folded into `docs/verification.md` by the integrator; this note is the
unit's own record._

Kernel sweep unit U9 (`plan/kernel-sweep-audit.md` section 3, "display
rounding"). Display only: no kernel function, no Kani harness, no Lean
theorem (the audit and `docs/proofs/README.md` say so for U9). The
rounding it shares is the kernel's already-proved `round_places`
(K-U2, law 40); the percentages it shares are `kernel::percent_of`
(K-U6).

## The API

```rust
// ledger-import::format (re-exported at the crate root)
pub fn round_display(amount: Decimal, places: u32) -> Decimal
```

Half away from zero to `places`, through
`ledger_core::kernel::round_places(..., Rounding::HalfAwayFromZero)`,
never a negative zero. It is the rounding half of
`ledger-import::report::format_amount`, lifted out so every renderer
shares it: `format_amount` and `amount_parts` call it at two places,
`ledger-import::pdf`'s percent column at one place,
`ledger-import::xlsx::exact_f64` at two places (before its exactness
check), `ledger-package::text`'s `money`/`share_text` at the amount's
own scale (the identity there: padding only, never rounding, as those
functions promise), and `ledger-gui::fmt` re-exports it for the pages.

## The percent rule (one rule for every percent)

A percent of a zero or negative whole is `None`, and the page shows
nothing (a KPI tile keeps no sub-line) or "—" (a table cell). That is
`kernel::percent_of`'s rule, and K-U9 adopts it everywhere:

- `ledger-gui::fmt::pct_change` returns `Option<Decimal>` (it was
  `Option<f64>`, dividing money amounts in binary floating point):
  `(now - prev) / prev * 100` to one place, half away from zero, via
  `percent_of`. A negative `prev` gave a signed percent before; now
  `None`.
- `ledger-gui::pages::profit::pct` (margins, shares of invoiced
  income): `percent_of` at one place, half to even — the books' margin
  mode, as `contribution_margin_pct` uses. A negative revenue base
  gave a percent before; now `None`, shown as "—".
- `ledger-gui::pages::reports`' change column: was
  `diff / base.abs() * 100`, a percent of a negative base with the sign
  hidden; now `percent_of(diff, base, ...)`, `None` for base <= 0.
  The "% of income" column likewise adds no column when total income
  is zero or negative (it added none for zero before).
- `ledger-gui::pages::capital::percent`: a member's share is a stored
  fraction, not a ratio of two amounts, so it stays exact decimal
  multiplication by 100 with no rounding; only a negative zero is
  dropped. No float was involved.

## Sites routed

- `ledger-import`: `report::format_amount`, `report::amount_parts`,
  `pdf` percent column, `xlsx::exact_f64` — through `round_display`.
- `ledger-package::text::money`, `share_text` — through
  `round_display` at their own scale (identity; see above).
- `ledger-gui::fmt::compact`: was `f64` (`try_into`, `{:.1}` prints).
  Now decimal division by 1,000 / 1,000,000 and `round_display`. Two
  displayed figures change, both f64 artefacts: 1,250,000 shows
  "$1.3M" (was "$1.2M": 1.25 in binary is slightly under), 10,500 shows
  "$11k" (was "$10k": float formatting rounds 10.5 to even).
- `ledger-gui::fmt::pct_change` and its caller
  `pages/overview::change` ("▲ 12% vs …" text and the 0.5% colour
  threshold): Decimal throughout.
- `pages/overview` runway months, `pages/cash` average net and runway,
  `pages/profit` spending row, `pages/reports` change / share / TSV,
  `books` monthly-burn average, `demo` platform fee: `round_dp(1|2)`
  through `round_display` (checked division where the old code divided
  raw under a guard).
- `books`' two share splits truncated to cents with `ToZero`: that is
  a leftover policy, not display rounding, so it now goes through
  `kernel::round_places(..., TowardZero)` as `truncate_cents`, with no
  change of value.

## Floats kept, with why

- `ledger-gui::charts` (`f(Decimal) -> f64`, `ticks`, sparklines,
  `pages/overview`/`cash` spark vectors, `widgets::Kpi::spark`): pixel
  coordinates and axis scales computed for layout only. Every hover
  and label that states a figure reads the original `Decimal`
  (`fmt::dollars`, `fmt::compact`); no printed amount is computed in
  a float.
- egui layout `f32`s (column widths, row heights), colour maths in
  `schemes.rs`, image sizes in `receipts.rs`: geometry and colour,
  never money.
- Time `f64`s (`app::since_update`, `widgets` timers,
  `pages/reports::pdf_done`, `pages/transactions::since_update`):
  egui's clock, never money.
- `fmt::bytes` (`n as f64`): a byte count for "12.3 KB", not money.
- `ledger-import::labels` geometry (`f64` millimetre/point layout),
  `asset_register`'s life-in-months cell (`f64::from(months)`: a
  count), `xlsx` entry-id cells (`as f64`: an id): layout and counts.
- `ledger-import::xlsx::exact_f64` (`to_f64`): the one sanctioned
  conversion — an xlsx cell is a binary double, and a value is
  written as a number only when it is whole cents of at most 15
  significant digits, exactly representable; anything else is text.
- `ledger-mcp::types` `visit_f64`: serde's visitor signature for
  refusing a float TIN, not a computation.
- `ledger-import::templates::equal_shares`: owner-percentage template
  (audit class (c)), last-absorbs so the shares sum to 100; not a
  display rounding of a money amount, left as is.

## Tests

- `crates/ledger-import/src/format.rs`: half away from zero at ties,
  never a negative zero.
- `ledger-gui::fmt` tests: `pct_change` decimal-or-none (zero and
  negative bases), `compact` decimal ties.
- `ledger-gui::pages::profit` tests: no percent of a negative base.
- `ledger-gui::pages::reports` tests: the change column has no
  percent from a zero or negative base; no "% of income" column for
  negative income.
- `crates/ledger-gui/tests/display_rounding.rs`: the rules through
  the public API, plus a scoped source check (no `round_dp` in the
  display files; no `f64` in `fmt`'s money half; `to_f64` only in
  `xlsx.rs`).

No golden changes. Displayed values that changed are the two
`compact` ties above and the percents of negative bases (now "—" /
no sub-line instead of a signed figure). One existing test encoded
the old negative-base percent (`ledger-gui/tests/view_model.rs`:
`pct_change(-100, -50)` was `Some(50.0)`); it now expects `None`,
per the rule above.
