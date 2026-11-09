# Claims: entry

Wanderbooks' claims for bookspec area **entry**: what each rule requires, where the engine enforces it, and which tests check it. The accounting text lives in bookspec's records (`rules/entry/`); here stay the option wanderbooks picks, its enforcement and tests, and the product rules that are only wanderbooks' (`WB-entry-<n>`). B-1, B-2, B-21, B-28 and B-43 are reduced.

Rules: B-1, B-2, B-21, B-28, B-43.

---

## B-1 Every entry is a double entry: debits equal credits

**bookspec:** `BS-entry-1`

**Pick (BS-entry-6):** `check-when-line-added`, the owner, 2026-10-13

**Claims:** yes

**Pick:** a line's account must exist and take postings when the line is added to a draft, not only when the entry posts: CHOICE (the owner, 2026-10-13).

**Enforced:** `kernel::check_lines` via `JournalEntry::validate` (two or
more lines, debits = credits exactly); `Ledger::add_line` for the account
check at line-adding time
(`a_line_on_an_unknown_or_placeholder_account_is_refused_when_added`).
**Checked:** Kani `check_lines_is_exact`, `accepted_entry_nets_to_zero`; Lean
`Accepted.net_zero`; property `prop_posted_entries_balance`.

### Product rules

None.

## B-2 A line's amount is positive; its side gives the direction

**bookspec:** `BS-entry-2`

**Claims:** yes

**Pick:** none (a CONVENTION). Wanderbooks refuses negative and zero amounts so that every line reads one way (2026-10-13, found by the formal pass).

**Enforced:** `kernel::check_lines` (`NegativeAmountLine`, `ZeroAmountLine`).
**Checked:** Kani `check_lines_is_exact`; test
`negative_amount_line_is_rejected`.

### Product rules

None.

## B-21 Exact amounts in the books; rounding only on returns

**bookspec:** `BS-entry-3`

**Claims:** yes

**Pick:** none (AUTHORITY). The books keep cents (and FX precision) exactly (R-2).

**Enforced:** `Decimal` money, `Amount` checked arithmetic, line range.
**Checked:** Kani `sums_are_exact`; bridge tests.

### Product rules

None.

## B-28 What new commands take: four-digit years, amounts a line can carry

**bookspec:** `BS-entry-4`

**Pick (BS-entry-7):** `refuse-rather-than-crash`, the owner, 2026-10-16

**Pick (BS-entry-8):** `bounds-bind-commands-never-replay`, the owner, 2026-10-17

**Claims:** yes

**Pick:** the bounds are bookspec's (dates 0001-01-01 through 9999-12-31, an amount at most `kernel::MAX_LINE_AMOUNT` = 1,000,000,000,000,000, scale at most `MAX_LINE_SCALE` = 10; a new foreign line's foreign amount and its FX rate take at most `MAX_LINE_SCALE` places each, so their product keeps its scale), and one run of the recurring entries creates at most 1,000 drafts (`MAX_RECURRING_RUN`). Provenance: CHOICE (P0 review round 3, 2026-10-16, finding S1: the
server stops for every session when a request panics, so no input may
reach one); CHOICE (P0 review round 4, 2026-10-17: posted is forever, so
the bounds are the commands', never replay's). A new asset's life past
9999 is refused at registration, the bound the life's last day held (P3
settled the plan's defect 22 question this way); ones an earlier version
took open, and their months after 9999-12 show as scheduled and never post.
The ten-place bound on a foreign amount and a rate is D-75 (the owner,
2026-11-08, "Cap them at 10 places"): the K-U1 review found that with a
26-place foreign amount `Decimal` rounded the product to 28 places before
the engine rounded it to the cent, booking a cent the exact product does
not reach (0.99999999999999999999999999 x 0.005 into USD). A journal's
older, finer foreign line replays as booked (`replay_add_line_fx`).

**Enforced:** `ledger_core::calendar`; `Ledger::try_create_entry`,
`add_line`, `add_line_fx` (the foreign amount and the rate, `ValueOutOfRange`,
never on replay), `validate_new_asset`, `dispose_asset`,
`schedules::validate_schedule`, `validate_recurring`,
`periods::validate_period_range`, `arap::validate_lines`,
`validate_due_date`, `payroll::check_payroll_ranges`, `payout_plan`,
`shares::check_shares`, `AssetBasis::check`, `reconciliation_report`;
`ledger-store` `parse_date` (replay: chrono's range), `parse_period`,
`last_day_of_month`, `record_statement`, `dispose_preview`,
`run_recurring`; `ledger-mcp` `parse_date`, `parse_period_mcp`,
`last_day_of_month`.
**Checked:** `ledger-core` `tests/spec_bounds.rs` (each bound and its
replay pair; `a_foreign_amount_and_a_rate_take_at_most_ten_places`),
`kernel_bridge/fx.rs` (`past_ten_places_the_reference_and_decimal_part_and_the_engine_refuses`,
`ten_places_each_is_exact_through_the_engine`), `calendar.rs`, `assets.rs`
`registration_rejects_date_overflow`, `invariants.rs`
`out_of_range_amount_is_rejected`; `ledger-store` `tests/bounds.rs`,
`tests/store.rs` `a_journaled_foreign_line_finer_than_ten_places_still_opens`;
`ledger-cli` `server.rs` `periods_are_a_year_and_a_month`,
`dates_are_four_digit_years_and_real_days`.

### Product rules

**WB-entry-1** Replay accepts every line an earlier version accepted.
The bounds of B-28 came after books were written, and posted is forever
(B-8): a journal an earlier wanderbooks wrote opens, in the writer and the
viewer alike, and replays to the same figures, forever. Replay applies
each check exactly as the version before the bound did: a journaled date
is any date chrono reads as `%Y-%m-%d` (chrono's own range, about
262,000 years either way, written "+10000-01-01" past 9999); a draft
line, a recurring template, a party's share, a contributor's basis, a
document line, a payment, a payroll run and disposal proceeds carry no
upper bound beyond what their own rules always said (positive, cents,
balanced, and so on); a schedule's months are 1 to 600 and an asset's
life ends on a date chrono holds. The checks split in two: the
command's (`Ledger::add_line`, `register_asset`, `AssetBasis::check`,
`shares::check_shares`, ...) and replay's (`replay_add_line`,
`replay_register_asset`, `AssetBasis::check_journaled`, `AssetBasis::check_legacy_journaled`,
`shares::check_shares_journaled`, ...), and every bound has its pair of
tests: the command refuses past it; replay of an old line past it is
accepted.
*Enforced:* for replay, `replay_add_line`, `replay_add_line_fx`,
`replay_register_asset`, `replay_correct_asset`,
`replay_dispose_asset`, `replay_set_asset_tax_basis`, `replay_set_asset_basis`,
`replay_set_entry_parties`, `replay_register_recurring`,
`replay_register_schedule` (`validate_schedule_shape`),
`replay_create_period` (`validate_period_order`), `replay_invoice`,
`replay_bill`, `replay_invoice_payment`, `replay_bill_payment`,
`replay_payroll_run`, `validate_new_asset_journaled`,
`validate_recurring_journaled`, `check_shares_journaled`,
`AssetBasis::check_journaled`, `AssetBasis::check_legacy_journaled`.
*Checked:* `ledger-core` `tests/spec_bounds.rs` (each bound and its
replay pair); `ledger-store` `tests/store.rs`
`books_an_earlier_version_wrote_open_to_its_figures` (every book in
`tests/fixtures/old-books`, written by main 188c90ea, opens in the writer
and the viewer and replays to exactly the figures main computed);
`ledger-gui` `tests/old_books.rs`.

**WB-entry-2** The full-TIN rule (B-26) is one of these bounds: P3's
detector finds a TIN across every Unicode space and dash and invisible
characters, P2's only across ASCII separators, the no-break space and a
few dashes, so replay finds one as P2 did
(`ledger_core::parties::replaying`, around every replayed line), and a
name, reason or fact P2 took opens
(`a_name_p2_took_replays_though_the_command_now_refuses_it`; the P3
data-safety review, 2026-10-21, found P3 refusing to open such books).
*Enforced:* `ledger_core::parties::replaying`. *Checked:*
`a_name_p2_took_replays_though_the_command_now_refuses_it`.

**WB-entry-3** What an earlier version left beside its journal is kept
for it the same way: its projection database, in its own schema's file,
is never migrated or written by a later version (B-27, "each projection
schema in a file of its own"), so the earlier version, gone back to,
opens its books as it left them.
*Enforced:* the projection schemas' separate files (B-27).
*Checked:* `ledger-store` `tests/store.rs`
`books_an_earlier_version_wrote_open_to_its_figures`.

**WB-entry-4** Derived figures refuse, never panic, and the server stays
up. Because replay accepts what the commands now refuse, every figure the
books derive is computed with checked arithmetic: a sum or product exact
arithmetic can't hold is refused with "too large to compute exactly"
(`TooLargeToCompute`), and a date past chrono's last day with
`DateOutOfRange`, at the point of computation (a draft's debits, a
document's total and balance, aging, net pay, a payroll plan, an entry's
money flow and its shares' resolution, the fixed asset register, an
amortization plan), never wrapped, rounded early or panicked on, and the
server stays up. Date walks reach chrono's last day without stepping past
it (an asset's depreciation in December 262142, an interval ending at
`NaiveDate::MAX`). A replayed line whose own figures overflow (a
recurring template whose debits Decimal can't sum: no version could
write it, and the last one panicked replaying it) is refused as the books
open, never a panic.
*Enforced:* derived: `Invoice::total`/`balance`,
`Bill::total`/`balance`, `AgingRow::total`, `ar_aging`, `ap_aging`,
`PayrollLine::net_pay`, `payroll_entry_plan`, `parties::entry_flow`,
`shares::target`, `shares::resolve`, `fixed_asset_register`,
`schedules::amortization_plan`, `month_end`, `depreciation_schedule`,
`analysis::intervals`, `form1099::vendor_part`, `k1::share_property`,
`ReceiptPolicy::entry_debits`.
*Checked:* the unit tests of `amortization_plan`, `intervals`,
`vendor_part` and `share_property`; `ledger-store` `tests/store.rs`
`every_figure_over_old_books_is_computed_or_refused`; `ledger-cli`
`tests/hostile_args.rs` (every tool with generated hostile
arguments and every resource with generated URIs: the server never
stops, and the books reopen after; the inputs that once panicked,
pinned; every tool over every old book).

## B-43 A draft waits on its evidence, and one that will never post can be discarded

**bookspec:** `BS-entry-5`


**Claims:** partly (bookspec's record states the evidence gate's principle, the draft state as a note and the discard; the gate's function, the policy settings, the states' names and the surfaces are wanderbooks product, below)

**Pick:** reversals stay posted and need no evidence of their own, their evidence being their original's: CHOICE D-47 (owner-adopted recommendation, 2026-10-26; stated in `BS-ledger-7`'s status note, and in B-20's claims). A draft's state as a note that moves no figure, and discarding a draft rather than posting and reversing it, are the CONVENTIONs of the record.

**Enforced:** `ledger_core::evidence::{requirement, Requirement}`,
`Ledger::{set_draft_state, discard_draft, posted_seq}`,
`Store::{set_draft_state, discard_draft, post_entry_checked}` (the gate the server posts a hand draft by; `Store::post_entry` stays the primitive commands and fixtures post through); events
`entry.draft_state_set`, `entry.draft_discarded` (feature `drafts_v2`).
**Checked:** Lean `Step.discard`, `Step.discard_keeps_books`,
`Step.discard_unblocks_close`; `ledger-core` `tests/invariants.rs`
(`discard_unblocks_close`); `ledger-store` `tests/store.rs`
(`draft_states_replay_and_project`, `drafts_v2_waits_for_the_upgrade`,
`posted_seq_is_the_posting_lines_seq`); `ledger-cli` `tests/drafts.rs` (`a_draft_waits_for_its_receipt_and_posts`,
`a_reversal_needs_no_receipt_of_its_own`, `gui_and_server_gate_agree`,
`stale_preview_refused`).

### Product rules

**WB-entry-5** One function, `evidence::requirement`, decides whether an
entry needs evidence: `Required` (with why) or `Exempt` (with the source
of the exemption). The store's gate, the server's previews and digest, the
viewer and Needs attention all call it; none decides on its own.
**Enforced:** `ledger_core::evidence::{requirement, Requirement}`,
`Store::post_entry_checked`. **Checked:** `ledger-cli` `tests/drafts.rs`
(`gui_and_server_gate_agree`).

**WB-entry-6** Exempt by origin, each with its source: a period's closing
entry (derived from the books), a schedule month's or a depreciation
run's posting (the schedule is the evidence), an asset's disposal, an
entry a document or a payroll command posts (it carries its own evidence
or is posted at once by design: an invoice, a bill, a payment, a
write-off, a payroll run or its tax remittance), and a reversal (its
evidence is its original's, D-47, D-12). Every other entry, hand entries
and recurring drafts included, is covered by the policy settings
(`policy.evidence.*`): required above the threshold, on a required
account, or as part of a split group; exempt only when every line is on an
exempt account.
**Enforced:** `ledger_core::evidence::requirement`. **Checked:**
`ledger-cli` `tests/drafts.rs` (`a_reversal_needs_no_receipt_of_its_own`,
`a_draft_waits_for_its_receipt_and_posts`).

**WB-entry-7** A draft has a state: `building` (the default) or
`awaiting_evidence` with a note. A state is a note on the draft; it moves
no line, no balance and no report, and a posted entry has none. Setting
one is journaled as `entry.draft_state_set` (feature `drafts_v2`).
**Enforced:** `Ledger::set_draft_state`, `Store::set_draft_state`.
**Checked:** `ledger-store` `tests/store.rs` (`draft_states_replay_and_project`,
`drafts_v2_waits_for_the_upgrade`).

**WB-entry-8** A discard spends no id, and what was recorded about the
draft (its vendor, instrument and parties) goes with it; the journal
still holds its lines and why it went (`entry.draft_discarded`, feature
`drafts_v2`).
**Enforced:** `Ledger::discard_draft`, `Store::discard_draft`. **Checked:**
`ledger-core` `tests/invariants.rs` (`discard_unblocks_close`);
`ledger-store` `tests/store.rs` (`draft_states_replay_and_project`).

**WB-entry-9** A refused post is `receipt_required`; the draft may then be
set `awaiting_evidence` with a note ("Figma invoice due"). A draft
awaiting evidence posts the moment the evidence is attached and the post
is confirmed against a fresh preview (a stale preview is refused). Every
refusal leaves the journal unchanged.
**Enforced:** `Store::post_entry_checked`. **Checked:** `ledger-cli`
`tests/drafts.rs` (`a_draft_waits_for_its_receipt_and_posts`,
`stale_preview_refused`).
