# Claims: asset

Wanderbooks' claims for bookspec area **asset**: what each rule requires, where the engine enforces it, and which tests check it. The accounting text lives in bookspec's records (`rules/asset/`); here stay the option wanderbooks picks, its enforcement and tests, and the product rules that are only wanderbooks' (`WB-asset-<n>`).

Rules: B-15, B-22, B-24, B-25.

---

## B-15 Depreciation: straight line, salvage never depreciated, partial months by days

**bookspec:** `BS-asset-1`

**Claims:** yes

**Pick:** `straight-line-by-days`: CHOICE (the owner, 2026-10-13; the engineer settled the arithmetic from ASC 360-10-35-4, 2026-10-20): straight line, partial months prorated by days, including a schedule through a mid-month date, with each cumulative figure rounded half away from zero to the currency's minor unit. The tax conventions (half-year, mid-month, mid-quarter) are not applied: they are the CPA's, per IRS Publication 946.

**Pick (BS-asset-5):** `straight-line-by-days`, the owner, 2026-10-13; the engineer settled the arithmetic from ASC 360-10-35-4, 2026-10-20.

**Enforced:** `assets::depreciation_schedule`, `assets::run_amount`,
`assets::catch_up_room`, `assets::depreciation_effects`,
`kernel::prorated_through`, `kernel::catch_up`, `kernel::true_up`,
`calendar::month_parts`, `Store::run_depreciation`,
`Store::dispose_preview` (`check_nothing_after_disposal`),
`Store::check_reverse_entry`, `Ledger::asset_acquired_date`,
`set_asset_acquisition` (`replay_set_asset_acquisition` for replay),
`check_correct_asset`, `dispose_asset` (strict: the acquired date),
`record_depreciation_run` / `check_depreciation_run`,
`accumulated_posted`, `live_depreciation_run`. **Checked:**
`schedule_sums_exactly_to_depreciable_amount`,
`salvage_value_is_never_depreciated`, `prop_depreciation_schedule_exact`,
`defect22_depreciation_through_mid_final_month_is_prorated_not_plugged`,
`defect22_final_month_drift_is_spread_by_days`,
`defect22_a_disposed_assets_schedule_stops_at_its_disposal_day`,
`defect22_a_disposal_in_the_final_month_posts_depreciation_by_days`,
`defect22_a_disposal_inside_a_posted_month_takes_back_the_extra_days`,
`a_run_reinstated_after_its_rerun_was_reversed_still_counts`,
`the_viewer_counts_a_run_reinstated_after_its_rerun_was_reversed`,
`a_reversal_dated_before_the_entry_it_reverses_is_refused`,
`a_disposal_is_taken_once_a_late_change_is_undone_after_its_month_closed`,
`a_disposal_whose_month_has_run_charges_what_is_still_due_in_its_entry`,
`every_order_of_runs_reversals_disposals_and_closes_keeps_the_books_right`,
`prop_any_sequence_keeps_the_books_right`,
`prop_the_viewer_reads_an_asset_as_the_store_does`,
`prop_run_amounts_are_each_months_run_amount`,
`jpy_depreciation_posts_whole_yen`, `a_skipped_month_is_caught_up_in_the_month_run`,
`a_reversed_depreciation_month_can_be_run_again`,
`a_run_reversed_after_its_month_is_caught_up_in_the_reversals_month`,
`reinstating_a_run_a_later_run_caught_up_is_refused`,
`a_disposal_before_a_later_months_run_is_refused_until_it_is_reversed`,
`a_disposal_before_a_later_reversal_of_an_earlier_run_is_refused`,
`a_disposal_entry_cannot_be_reversed`,
`prop_the_final_month_takes_the_remainder_by_its_days`,
`depreciation_effects_read_each_entry_in_its_month`,
`prop_depreciation_is_monotone_and_bounded`,
`prop_disposed_schedule_equals_what_disposal_computed`,
`prop_month_end_schedules_match_the_legacy_arithmetic_except_half_cent_ties`;
`the_charge_never_falls_and_is_continuous_over_real_calendars`; Kani
`prorate_parts_sum_to_total`, `prorate_last_part_absorbs_the_remainder`,
`a_months_charge_is_within_half_a_cent`,
`a_months_charge_in_tenths_is_within_half_a_cent`,
`the_last_months_charge_is_within_half_a_cent`,
`the_last_months_charge_in_tenths_is_within_half_a_cent`,
`prorated_through_refuses_malformed_input`,
`catch_up_never_overtakes_the_schedule`,
`catch_ups_is_every_suffixs_catch_up`, `true_up_reaches_the_target`;
Lean `Prorate.kernel_parts_total`, `Prorate.kernel_parts_nonneg`,
`Prorate.part_near`, `Prorate.cum_near`, `Prorate.chargeMid_eq_cum`,
`Prorate.chargeLast_eq_cum`, `Prorate.chargeLast_start`,
`Prorate.chargeLast_mono`, `Prorate.cum_mono`, `Prorate.cum_le`,
`Prorate.cum_full`, `Prorate.cum_dvd` (bookspec's Lean, D-69);
`ledger-core` `tests/spec_asset_basis.rs` (the in-service and acquired
dates, `accumulated_posted` and its edges,
`a_reinstated_run_counts_and_is_live`); `ledger-store`
`tests/asset_tags.rs` (`runs_replay_reload_and_verify_catches_a_tampered_run`,
`the_viewer_and_writer_agree_on_a_bad_depreciation_posted_payload`);
`ledger-cli` `tests/spec_assets.rs` (`a_reversed_run_is_not_posted`,
`list_assets_book_value_is_what_the_books_carry`).

### Product rules

**WB-asset-1** The bounds `BS-asset-1` states (statements 25-26: a new link to
an entry dated after the in-service date or the disposal is refused, as is
a new in-service date before the linked entry's date, and a disposal dated
before the acquired date is refused) are the commands', never replay's
(B-28): a journal an earlier version wrote replays as written, whatever
those dates say. (That depreciation starts on the in-service date and that
the register holds the asset from the acquired date are `BS-asset-1`
statements 23-24.)
*Enforced:* `set_asset_acquisition` / `replay_set_asset_acquisition`,
`Ledger::asset_acquired_date`, `dispose_asset`, `Ledger::set_asset_acquisition`.
*Checked:* `ledger-core` `tests/spec_asset_basis.rs` (each bound and its
replay pair); `ledger-store` `tests/bounds.rs`.

**WB-asset-2** The store and the viewer read every run the books recorded
(`Ledger::depreciation_runs`: each `asset.depreciation_posted`, as replay
records it), never a month's latest: a run re-instated after its month was
run again and that run reversed is the month's run.
*Enforced:* `Ledger::depreciation_runs`, `live_depreciation_run`.
*Checked:* `the_viewer_counts_a_run_reinstated_after_its_rerun_was_reversed`,
`prop_the_viewer_reads_an_asset_as_the_store_does`.

## B-22 Expense or capitalize (book policy, mirrors tax)

**bookspec:** `BS-asset-2`

**Claims:** no (no engine enforcement exists: the policy is the books' own AGENTS.md, and the de minimis and 12-month elections are made on the return)

**Pick:** none (AUTHORITY; procedural). The books' own AGENTS.md sets the policy; the de minimis and 12-month elections are made on the return.

**Enforced:** bookkeeper policy (books' AGENTS.md), not the engine.

### Product rules

None.

## B-24 Asset tags: one label per asset, never reused

**bookspec:** `BS-asset-3`

**Claims:** yes

**Pick (BS-asset-3):** `tag-and-never-reuse`, the owner, 2026-10-15.

**Pick:** `tag-and-never-reuse`: CHOICE (the owner, 2026-10-15): each asset may carry one tag, generated `PREFIX-0001` or the human's own, unique ignoring case and never reused, and the register reconciles to the cost and accumulated depreciation accounts.

**Enforced:** `Ledger::set_asset_tag` / `check_asset_tag_for` /
`check_asset_tag` (the tag already taken is refused, the retag keeping the old tag reserved),
`Ledger::set_asset_serial` / `check_asset_serial`, `assets::check_asset_tag`,
`assets::check_serial`, `reports::fixed_asset_register`.
**Checked:** `ledger-core` `tests/spec_asset_tags.rs`; `ledger-store`
`tests/asset_tags.rs` (`a_journaled_tag_taken_by_another_asset_is_refused_on_open`,
`a_retagged_assets_old_tag_is_never_reused`);
`ledger-cli` `tests/spec_assets.rs`.

### Product rules

**WB-asset-3** A tag and a serial number are journaled (`asset.tagged`,
which keeps the tag the asset had before; `asset.serial_set`), and the
printer makes an Avery 5160 label sheet from the register's tags.
*Enforced:* `Store::set_asset_tag`, `Store::set_asset_serial`;
`books.label_asset`.
*Checked:* `ledger-store` `tests/asset_tags.rs`
(`runs_replay_reload_and_verify_catches_a_tampered_run`);
`ledger-cli` `tests/spec_assets.rs`.

## B-25 Tax figures beside book value: the §723 basis, the depreciable basis, the §704(c) built-in gain

**bookspec:** `BS-asset-4`

**Claims:** yes

**Pick:** `register-carries-tax-figures` and the D-41 options: the register carries the tax figures beside book value (CHOICE, the owner, 2026-10-15); for property a member contributed from personal use, the depreciable basis is the lesser of its fair market value and the member's adjusted basis (the MAY of `BS-asset-4`, taken by analogy to Treas. Reg. §1.168(i)-4(b)(1), which governs a taxpayer converting its own property; the choice is wanderbooks', the CPA to confirm at year end); the §704(c) method is `traditional`, recorded as a choice the CPA confirms; the contributed property's values on the contribution date are the books' recorded amounts (CHOICE D-41, the owner, 2026-10-20).

**Pick (BS-asset-6):** `lesser-of-value-and-basis`, the owner, D-41, 2026-10-20.

**Pick (BS-asset-7):** `books-recorded-amounts`, the owner, D-41, 2026-10-20.

**Pick (BS-asset-8):** `register-carries-tax-figures`, the owner, 2026-10-15; D-41, 2026-10-20.
**Pick (BS-asset-9):** `traditional`, the owner, D-41, 2026-10-20; recorded as a choice the CPA confirms.

**Enforced:** `kernel::basis_figures` (Kani `basis_figures_laws`),
`AssetBasis::check` (a purchase takes no contribution facts; a
contribution needs the adjusted basis and the fair market value, each
non-negative, in cents, at most one quadrillion) and its replay pairs
`check_journaled` and `check_legacy_journaled`, `Ledger::set_asset_basis`
/ `check_asset_basis` / `replay_set_asset_basis` /
`replay_set_asset_tax_basis`, `Asset::basis_figures`,
`reports::fixed_asset_register` (per-row tax fields,
`total_tax_basis_723`, `total_depreciable_basis`, `missing_basis`);
`ledger-store` `Store::set_asset_basis` (the journal feature
`asset_basis`); `ledger-mcp` `books.set_asset_tax_basis` and
`books.register_asset` (a contribution's fair market value is required,
never assumed). **Checked:** `ledger-core` `tests/spec_asset_basis.rs`,
`tests/kernel_bridge/` (`basis_figures_agree`), `tests/properties.rs`
(`prop_basis_figures_laws`); `ledger-store` `tests/asset_tags.rs`;
`ledger-cli` `tests/spec_assets.rs`.

### Product rules

**WB-asset-4** A record made before P3 (`asset.tax_basis_set`, with the
old names `cost` and `carryover`) has no fair market value recorded: it
reads as the book cost, which reproduces that version's figures exactly,
and Needs attention asks for the value.
*Enforced:* `replay_set_asset_tax_basis`, `AssetBasis::check_legacy_journaled`;
`ledger-gui` Needs attention.
*Checked:* `ledger-core` `tests/spec_asset_basis.rs`
(`legacy_tax_basis_set_lines_replay_to_todays_figures`); `ledger-store`
`tests/asset_tags.rs`.

**WB-asset-5** The tool surface never assumes a contributed property's
fair market value and never accepts a tax figure without the facts behind
it: `books.register_asset` requires the contribution's fair market value,
and `books.set_asset_tax_basis` records the rule, the member's adjusted
basis, the fair market value at contribution, the §704(c) method if chosen,
and who supplied them.
*Enforced:* `ledger-mcp` `books.register_asset`, `books.set_asset_tax_basis`;
`AssetBasis::check`.
*Checked:* `ledger-cli` `tests/spec_assets.rs`; `ledger-core`
`tests/spec_asset_basis.rs`.
