# Claims: asset

Wanderbooks' claims for bookspec area **asset**: what each rule requires, where the engine enforces it, and which tests check it. Being reduced to the engine's pick and its evidence as P14 unit S-D lands; the accounting text moves to bookspec's `spec/asset.md`.

Rules: B-15, B-22, B-24, B-25.

---

## B-15 Depreciation: straight line, salvage never depreciated, partial months by days

**bookspec:** `BS-asset-1`

**Status:** CONVENTION (book method); AUTHORITY for tax conventions, which
wanderbooks does not apply

OpenStax §11.3 (cited): "Straight-line depreciation is a method of
depreciation that evenly splits the depreciable amount across the useful
life of the asset," with "(Cost – salvage value)/Useful life"; partial
periods: "A common method is to allocate depreciation expense based on the
number of months the asset is owned in a year." wanderbooks prorates
partial months by days, including a schedule through a mid-month date
(CHOICE, the owner 2026-10-13). ASC 360-10-35-4 (read 2026-10-14,
`sources/fasb-asc.md`) prescribes no method: cost less salvage is spread
over the useful life "in a systematic and rational manner," as equitably
as possible to the periods served; straight line by days is one such
allocation.
Tax conventions (half-year, mid-month, mid-quarter): IRS Pub. 946
(`sources/irs-depreciation.md`), for the CPA.

The rule, exactly (P3, defect 22; the engineer settled it from ASC
360-10-35-4, 2026-10-20):

1. The life is `[in-service date, in-service date + life months)` (chrono
   clamps a day past a month's end: Jan 31 plus one month is Feb 28), cut
   into the calendar months it touches.
2. Through day `k` of a month but the life's last, the depreciation is
   (cost − salvage) / life × (the months before, each as days held over
   days in the month, + `k` / days in this month): the monthly rate by
   days.
3. The life's last month takes what the months before it leave, evenly
   over its days: through its last day the depreciation is exactly cost
   − salvage, and through an earlier day of it, prorated by days, never
   plugged to the whole. (Without clamping its daily rate is the first
   month's: the remainder is that month's missing days.) Before P3 any
   schedule through a day of the final month, and every disposed asset's,
   plugged to the whole: a disposal in the final month posted too much
   depreciation and too large a gain.
4. Each cumulative figure is rounded half away from zero to the
   currency's minor unit (cents; whole yen; a thousandth of a dinar;
   ISO 4217, `sources/iso-4217.md`), in exact integer arithmetic
   (`kernel::prorated_through`); a month's amount is the difference of
   consecutive ones. Through every month end this is the per-month
   running total books posted before P3, except where the exact figure is
   a half-cent tie, which the old 28-digit Decimal quotients could round
   either way; P3 rounds it up, exactly, moving that cent to the month
   before.
5. A disposed asset depreciates through its disposal day, held
   (inclusive): its schedule, accumulated depreciation and book value
   stop there, and equal what the disposal took. The disposal's book
   value is cost less the schedule through that day whatever the runs
   posted: depreciation still due posts as the final depreciation;
   depreciation the disposal month's run (dated its end) posted past the
   day is credited back to depreciation expense in the disposal entry
   (`excess_depreciation`); and the disposal clears what the runs carry,
   so the asset leaves the accumulated depreciation account at zero. A
   disposal is refused unless every run carries at the end of the
   disposal's month what it carries now (the disposal clears what they
   carry now): while a run of a later month stands (reverse it first:
   credited back at the disposal day, it would leave the disposal month's
   accumulated depreciation a debit and its expense short), or while a
   reversal (or re-instatement) of an earlier run dated after the month
   isn't undone (the disposal would charge that run again, and the
   reversal take it away only later; undone on any open day, so a closed
   month never blocks a disposal for ever). A change a run carries only
   at later month ends (a later month's run reversed after its month) is
   that run's error and its correction, dated when made, and the disposal
   leaves it as it is.
6. A month's run posts what the schedule says is due through the
   month's end, less what the books carry then, but never so much that
   the books would carry more than the schedule at a later month end
   (`kernel::catch_up`). What the books carry is read entry by entry, each
   in its own month (`assets::depreciation_effects`): a run adds its
   amount in its month, a reversal of it takes it away in the reversal's
   month. So a month skipped, or a run reversed, is caught up in the month
   the run is made (plan/served.md P3, settled in design, 2026-10-20: an
   immaterial miss; a material one is restated under ASC 250, a
   correction for a human and the CPA), and a month whose run was
   reversed after it ended is never run twice (it carried the month at
   its end; the run of the reversal's month catches it up). A month a
   later run caught up, or that rounds to nothing, posts nothing (an entry
   carries no zero line); a run after the life posts what is still due.
   A reversed run is un-posted (its month due again, when reversed within
   it); re-instating one (reversing its reversal) is refused when it would
   carry the books past the schedule (a later run caught its month up:
   `assets::catch_up_room`); a disposal, or a run its disposal took as
   final, can't be reversed; and a run, or a reversal of one, can't be
   reversed on a day before its own (a correction comes after what it
   corrects, B-9: a run of March reversed on Jan 31 would leave January
   and February with negative depreciation; commands only, journals
   replay as written). The viewer and the store read every run the books
   recorded (`Ledger::depreciation_runs`: each `asset.depreciation_posted`,
   as replay records it), not a month's latest (a run re-instated after its
   month was run again and that run reversed is the month's run).
7. Two dates (P3, defect 13). Depreciation starts on the day the asset is
   placed in service, never the day it was bought: ASC 360-10-35-4
   spreads cost over the periods the asset serves, and an asset serves no
   period before it is in service. (For tax, Pub. 946's "ready and
   available for a specific use", `sources/irs-partnership-basis.md`, is
   the CPA's.) The register holds an asset from the day the company
   acquired it: the date of the entry it is linked to (its purchase or
   contribution), never later than the in-service date, or the in-service
   date when no entry is linked (settled in design, 2026-10-20: "an
   asset's acquired date comes from the entry it is linked to"; one
   source of truth, so the register and the general ledger can't
   disagree, B-24). A new link to an entry dated after the in-service date
   or the disposal is refused, as is a new in-service date before the
   linked entry's date; a disposal can't come before the acquired date (a
   command's bound; replay accepts what earlier versions did, B-28), and
   one before the in-service date takes no depreciation.
8. Posted and scheduled. What the schedule says is due through a date
   (`assets::accumulated_scheduled_through`, the tools'
   `accumulated_scheduled_through`) and what the books carry
   (`Ledger::accumulated_posted`: the runs read entry by entry as rule 6
   reads them, each on its own day: a run adds its amount, a reversal of
   it takes it away, a reversal of that reversal adds it back, B-8, B-9;
   zero once the asset is disposed, its disposal entry having cleared it)
   are two figures, never one: book value is cost less the posted figure
   (`assets::scheduled_book_value` is the schedule's own, for the
   schedule only). An asset's depreciation expense over a span is its
   runs' (`Ledger::depreciation_posted_between`) plus its disposal entry's
   lines on its expense account (rule 5): the P&L's, the cash flow
   reconciliation's "Depreciation" (B-14 rule 8), the tax-basis K-1's
   open item (B-36) and the IIF's all show that one figure
   (`depreciation_is_one_figure_everywhere`). `books.list_assets` as of a
   day says "disposed" only for a disposal dated by then, as `held` and
   the figures do (B-33; before the seam review a later disposal showed
   on an earlier day).

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
`Prorate.chargeLast_mono`, `Prorate.cum_mono` (`docs/verification.md`);
`ledger-core` `tests/spec_asset_basis.rs` (the in-service and acquired
dates, `accumulated_posted` and its edges,
`a_reinstated_run_counts_and_is_live`); `ledger-store`
`tests/asset_tags.rs` (`runs_replay_reload_and_verify_catches_a_tampered_run`,
`the_viewer_and_writer_agree_on_a_bad_depreciation_posted_payload`);
`ledger-cli` `tests/spec_assets.rs` (`a_reversed_run_is_not_posted`,
`list_assets_book_value_is_what_the_books_carry`).

## B-22 Expense or capitalize (book policy, mirrors tax)

**bookspec:** `BS-asset-2`

**Status:** AUTHORITY (`sources/irs-capitalization.md`)

Treas. Reg. §1.263(a)-1(f)(1)(ii)(D) sets $500 per invoice or item for a
taxpayer without an applicable financial statement "or other amount as
identified in published guidance"; Notice 2015-82 raised it to $2,500 for
taxable years beginning on or after January 1, 2016. The 12-month rule:
§1.263(a)-4(f)(1). The election is made yearly on the return.

**Enforced:** bookkeeper policy (books' AGENTS.md), not the engine.

## B-24 Asset tags: one label per asset, never reused

**bookspec:** `BS-asset-3`

**Status:** CHOICE (the owner, 2026-10-15)

Each fixed asset may carry a tag, the label stuck on the thing itself
(generated `PREFIX-0001` or the human's own: 1–32 ASCII letters, digits,
`- _ . /`), and the manufacturer's serial number. Tags are unique ignoring
case and are never reused by another asset, even after a retag or a
disposal, so an old label can never point at the wrong thing. Both are
journaled. The fixed asset register lists every asset acquired by a date
(B-15) and reconciles to the general ledger's cost and accumulated
depreciation accounts: cost per register and per books
(`unregistered_cost`), accumulated depreciation by schedule and per
books (`unposted_depreciation`: run depreciation for it), and from the
assets' runs and per books (`unattributed_accumulated`: an entry made by
hand). Book value on the register is cost less the depreciation posted,
the books' figure, never the schedule's (P3, defect 13).

**Enforced:** `Ledger::set_asset_tag` / `check_asset_tag_for`,
`assets::check_asset_tag`, `assets::check_serial`. **Checked:**
`ledger-core` `tests/spec_asset_tags.rs`; `ledger-store`
`tests/asset_tags.rs`; `ledger-cli` `tests/spec_assets.rs`.

## B-25 Tax figures beside book value: the §723 basis, the depreciable basis, the §704(c) built-in gain

**bookspec:** `BS-asset-4`

**Status:** AUTHORITY (`sources/irs-partnership-basis.md`); CHOICE
(the owner, 2026-10-15: the register carries tax basis); CHOICE D-41
(the owner, 2026-10-20: the depreciable basis of property a member
contributed from personal use is the lesser of its fair market value and
the member's adjusted basis, the CPA to confirm at year end; the
§704(c) method is traditional, recorded as a choice the CPA confirms;
the contributed property's values on the contribution date are the
books' recorded amounts)

Book value and tax figures are different numbers for the same asset. The
book value of a member's contribution is its fair market value (Treas.
Reg. §1.704-1(b)(2)(iv)(d)(1), cited; Treas. Reg. §1.704-3(a)(3)(i):
"book value is equal to fair market value at the time of contribution").
Three tax figures, each named for what it is, follow from the rule and
the facts recorded (`asset.basis_set`: the rule, the contributing
member's adjusted basis, the fair market value at contribution, the
§704(c) method if chosen, and who gave them, B-29):

- **Purchase** (`purchase`): "The basis of property shall be the cost of
  such property" (IRC §1012(a)); depreciable basis is the cost. Not
  §704(c) property: no §723 basis and no built-in gain.
- **§723 basis** of contributed property: "the adjusted basis of such
  property to the contributing partner at the time of the contribution"
  (IRC §723; Pub. 541: "the partnership's basis for determining
  depreciation, depletion, gain, or loss"). Adjusted basis is cost plus
  improvements, less depreciation allowed but not less than allowable
  (IRC §1016(a)(1)-(2)). wanderbooks does not model §721(b) gain (an
  investment-company partnership only), so the §723 basis is the
  member's adjusted basis.
- **Depreciable basis:** for business property contributed
  (`carryover_723`), the §723 basis; for property a member contributed
  from personal use (`personal_use_conversion`), "the lesser of its fair
  market value or its adjusted depreciable basis" (Treas. Reg.
  §1.168(i)-4(b)(1); Pub. 946, "Property changed from personal use";
  D-41).
- **§704(c) built-in gain or loss at contribution:** fair market value
  less the §723 basis ("the excess of the property's book value over the
  contributing partner's adjusted tax basis upon contribution", Treas.
  Reg. §1.704-3(a)(3)(ii)); fixed at contribution, so correcting the book
  cost later never moves it. A built-in loss counts only for the
  contributing member (IRC §704(c)(1)(C)). The §704(c) method
  (traditional, traditional with curative allocations, remedial: Treas.
  Reg. §1.704-3(b)-(d)) is chosen property by property (§1.704-3(a)(2)).

The figures are at contribution (or purchase): tax depreciation reduces
the basis afterwards (§1016(a)(2)) and the built-in gain or loss "is
thereafter reduced" (§1.704-3(a)(3)(ii)); both are the CPA's (MACRS,
conventions, elections: `sources/irs-depreciation.md`). The facts are
kept, never the figures: the figures are derived when read, under the
rule in force, so a later change to a rule changes the reports without
refusing a line an earlier version wrote (B-28). A record made before
P3 (`asset.tax_basis_set`, with the old names `cost` and `carryover`)
has no fair market value recorded: it reads as the book cost, which
reproduces that version's figures exactly, and Needs attention asks for
the value. wanderbooks' book depreciation is GAAP straight line, not
§704(b) book depreciation, which must keep the tax depreciation's ratio
(Treas. Reg. §1.704-1(b)(2)(iv)(g)(3), cited): neither is the other.

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

