# Claims: ledger

Wanderbooks' claims for bookspec area **ledger**: what each rule requires, where the engine enforces it, and which tests check it. The accounting text lives in bookspec's records (`rules/ledger/`); here stay the option wanderbooks picks, its enforcement and tests, and the product rules that are only wanderbooks' (`WB-ledger-<n>`).

Rules: B-3, B-4, B-5, B-8, B-9, B-10, B-20, B-23, B-29, B-31, B-32, B-44, B-45.

---

## B-3 One money through the ledger; foreign amounts at the transaction-date rate

**bookspec:** `BS-ledger-1`

**Pick (BS-ledger-14):** `book-in-the-smallest-unit`, the owner, 2026-10-13

**Claims:** yes

**Pick:** a foreign line is booked in the base currency's smallest unit (ISO 4217 minor units, `sources/iso-4217.md`), half away from zero: CHOICE (the owner, 2026-10-13), after the GnuCash cross-check found wanderbooks kept $108.34567 where GnuCash booked $108.35. Lines booked before then replay as booked (WB-entry-1).

**Enforced:** `Ledger::add_line` (base currency only, no fraction of the
minor unit), `Ledger::add_line_fx` (base amount = foreign amount × rate,
rounded once to the minor unit by `kernel::fx_booked_minor`; original kept;
a foreign amount or a rate past ten places is refused, D-75, B-28),
`Ledger::post_entry` (every
line, whatever made it: payroll, closes, disposals; `FinerThanMinorUnit`,
since 2026-10-13 after a payroll run posted $65.27794). Journals from
before replay as booked (`replay_post_entry`). A new asset's cost and
salvage are in the minor unit too (P3: every depreciation and disposal
line derives from them; `validate_new_asset`), and depreciation and daily
schedules round to it (B-15, B-16); assets an earlier version took finer
replay as journaled. **Checked:**
`an_asset_cost_finer_than_a_cent_is_refused`,
`a_foreign_line_is_booked_in_whole_base_units`,
`no_entry_posts_a_fraction_of_the_minor_unit`,
`foreign_currency_line_is_rejected`, `fx_overflow_is_an_error`;
`kernel_bridge/fx.rs` (the exact reference, ten places each); Kani
`proofs::fx::*`; Lean `Fx.booked_near`, `Fx.booked_whole`, `Fx.booked_pos`;
the GnuCash oracle.

### Product rules

None.

## B-4 Five elements, closed list

**bookspec:** `BS-ledger-2`

**Pick (BS-ledger-15):** `child-has-its-parents-type`, the owner, 2026-10-13

**Claims:** yes

**Pick:** a child account has its parent's type: CHOICE (the owner, 2026-10-13).

**Enforced:** `AccountType` (five variants); `Ledger::add_account`
(`ParentTypeMismatch`). **Checked:** `a_child_account_has_its_parents_type`.

### Product rules

None.

## B-5 Normal balances

**bookspec:** `BS-ledger-3`

**Claims:** yes

**Pick:** none (a CONVENTION; wanderbooks follows it as stated).

**Enforced:** `AccountType::normal_side`, `kernel::normal_balance`.
**Checked:** Kani `normal_balance_is_signed`, `normal_side_increases`; Lean
`balance_after_line`.

### Product rules

None.

## B-8 Posted is forever: the book is written only forward

**bookspec:** `BS-ledger-4`

**Claims:** yes

**Pick:** none (AUTHORITY).

**Enforced:** posted entries immutable; `ledger_store::format`
(`JournalFormat::admit`, `gate`): a record in a format version this
version does not know is refused as newer, neither misread nor skipped,
and never called corrupt (statement 4). **Checked:** Lean `Steps.extends`; property `prop_posted_entries_immutable`; `ledger-store`
`tests/format.rs` (`old_reader_gets_newer_journal`,
`a_newer_line_the_next_line_chains_onto_is_newer_and_intact`,
`damaged_journals_are_still_corrupt`).

### Product rules

**WB-ledger-1** The journal is append-only and hash-chained, so that a change to any journaled record is detected.
*Enforced:* the journal is append-only and hash-chained.
*Checked:* store `journal_tamper_is_detected`.

## B-9 A mistake is corrected by an opposite entry, never erased

**bookspec:** `BS-ledger-5`

**Pick (BS-ledger-16):** `on-or-after-the-original`, the owner, 2026-11-08, wanderbooks D-73

**Claims:** yes

**Pick:** none (AUTHORITY).

**Enforced:** `Ledger::reverse_entry` (same lines, sides flipped, linked by
`reversal_of`). **Checked:** Kani `reversal_mirrors`, `reversal_cancels`;
Lean `accepted_reversal_iff`, `movement_with_reversal`.

**Pick (D-73, the owner, 2026-11-08):** every reversal is dated on or after
the entry it reverses, for every kind of entry (it was enforced only for
document reversals and for schedule and depreciation runs): a correction
comes after what it corrects. The same day is allowed. New commands only:
a journal an earlier version wrote may hold a reversal dated before its
original, and it replays as it was (B-28).
**Enforced:** `Ledger::check_reversal` (SCENARIO 6) refuses with
`ReversalBeforeOriginal`, after the schedule double-post check (SCENARIO 5); the
store's `check_reverse_entry` and so `books.reverse_entry` (preview and
post alike) and every document void go through it, and replay is exempt
(`crate::parties::is_replaying`). **Checked:** store
`a_plain_entrys_reversal_is_not_dated_before_it`,
`a_journal_with_an_earlier_dated_reversal_opens_as_it_was`; core
`reversal_before_original_refused_for_documents`,
`a_reversal_dated_before_its_run_matches_the_trial_balance` (replayed);
the model oracle `the_ledger_matches_its_model` (its `reverse` requires
the date, as BS-ledger-5 and its CHOICE BS-ledger-16 do, and the Lean
model's `reverse` step since L-G1).

### Product rules

None.

## B-10 An entry is reversed at most once; a closing entry never

**bookspec:** `BS-ledger-6`

**Claims:** yes

**Pick (BS-ledger-6):** `reversible-once`, the owner, 2026-10-13.

**Pick:** `reversible-once`: CHOICE (the owner, 2026-10-13).

**Enforced:** `Ledger::check_reversal`. **Checked:**
`an_entry_is_reversed_at_most_once`, `a_closing_entry_is_never_reversed`.

### Product rules

None.

## B-20 Every transaction has supporting documents; records are kept

**bookspec:** `BS-ledger-7`

**Pick (BS-ledger-17):** `reversal-evidence-is-originals`, D-47, the owner, 2026-10-26

**Claims:** partly (the engine does not enforce it; the receipt policy of the tool layer does)

**Pick:** a reversal's evidence is its original's (a reversal needs no evidence of its own): CHOICE D-47 (the owner, 2026-10-26; options: a reversal's evidence is its original's / a reversal needs its own). The rest is AUTHORITY; a procedural rule.

**Enforced:** the MCP receipt policy (not the engine; R-13).
**Checked:** `ledger-cli` `tests/drafts.rs`
(`a_draft_waits_for_its_receipt_and_posts`,
`a_reversal_needs_no_receipt_of_its_own`); `ledger-store`
`tests/store.rs` (`draft_states_replay_and_project`,
`drafts_v2_waits_for_the_upgrade`); bookspec example
`ledger-a-reversal-needs-no-evidence-of-its-own` (BS-ledger-17, the option
picked: a reversal of a 40.00 entry under a policy of over 25.00 posts at once
and `ledger_core::evidence::requirement` exempts it as a reversal); the example
`ledger-a-reversal-carries-its-own-evidence` names the option not picked and is
skipped.

### Product rules

None.

## B-23 Control roles: one account each, of the right kind, moved only when empty

**bookspec:** `BS-ledger-8`

**Pick (BS-ledger-18):** `assignment-is-a-fact-in-the-books`, the owner, 2026-10-15

**Pick (BS-ledger-19):** `refuse-rather-than-renumber`, the owner, 2026-10-13

**Claims:** yes

**Pick:** the assignment of a role to an account is journaled, so replay never depends on a setting outside the journal: CHOICE (the owner, 2026-10-15); the engine refuses a role account that is not postable or of the role's type rather than renumber the chart: CHOICE (the owner, 2026-10-13). Receivables and payables are different (B-38, 2026-10-23): their documents read their figures from their lines.

**Enforced:** `Ledger::check_controls`, `Ledger::assign_role` /
`check_assign_role`, `Ledger::check_renumber_account` (an assigned role
frees its code); the control guard and `Ledger::control_check` (B-38).
**Checked:** `ledger-core` `tests/spec_roles.rs`, `tests/spec_documents.rs`;
`ledger-store` replay of `role.assigned`; bookspec examples
`ledger-a-role-on-a-header-or-wrong-typed-account-is-refused` (BS-ledger-19,
the option picked: the receivables role on a header account and the payables
role on a postable revenue account are refused through `Store::assign_role`
and `Ledger::check_assign_role`'s `ControlAccountMismatch`, and the chart is
as it was) and `ledger-a-role-assignment-is-a-fact-in-the-books`
(BS-ledger-18, the option picked: the receivables role assigned to 1300 is
journaled by `Store::assign_role`, read back by the `role_account` query
from the books alone, and found again by the store's replay), run by
`every_bookspec_example_holds` and
`the_runner_reports_every_claimed_rule_green`.

### Product rules

**WB-ledger-2** Invoices, bills, payroll, and closes post to accounts the engine picks by role: receivables, revenue, payables, general expense, retained earnings, salary and payroll tax expense, and five payroll withholdings. A role uses its default code (1200, 4000, 2000, 6999, 3999, 6100, 6110, 2110–2150) until the human assigns it one of the chart's own accounts; the assignment is journaled (`role.assigned`).
*Enforced:* `Ledger::assign_role` / `check_assign_role`. *Checked:* `ledger-store` replay of `role.assigned`.

**WB-ledger-3** The account at a role's default code is the role's control account only once a document posts there (or the role is assigned to it): before that, a chart's own 1200 Equipment or a card at 2000 is the chart's.
**Enforced:** `Ledger::check_controls`, the control guard, `Ledger::control_check`; `tests/spec_roles.rs`, `tests/spec_documents.rs`.
**Checked:** `tests/spec_roles.rs`, `tests/spec_documents.rs`.

**WB-ledger-4** Until the balance of a receivables or payables control account moves with its documents (plan P4b S7), a receivables or payables role whose control account holds a balance stays where it is, and the refusal says so: it names no hand entry, which the control guard would refuse.
**Enforced:** `Ledger::check_assign_role`, `Ledger::check_renumber_account`,
the control guard. **Checked:** `tests/spec_roles.rs`,
`tests/spec_documents.rs`.

## B-29 The books keep how they are kept

**bookspec:** `BS-ledger-9`

**Claims:** yes

**Pick (BS-ledger-9):** `facts-in-the-books`, the owner, 2026-10-16.

**Pick:** `facts-in-the-books`: CHOICE (plan/served.md I-3, owner decisions of 2026-10-16: "the journal holds all books state"; light governance: decisions are recorded with who made them and when, not as paperwork). IRS Pub. 538 and the rules of B-1 to B-28 say what each setting means.

**Enforced:** `ledger_core::settings` (`check`, `Settings::check_changes`,
`check_whole`, `shape`, `apply`), `ledger_core::conventions` (`check_new`,
`check_decision`, `check_open_item`, `split`), `Ledger::check_settings`,
`check_convention`; `ledger-store` `Store::change_settings`,
`set_convention`, `record_decision`.
**Checked:** `ledger-core` settings and conventions unit tests;
`ledger-store` `tests/settings.rs` (`settings_replay_is_pure`, 256 cases
per check, 2,000 by hand, with posts between records and every record
checked to change no figure or entry;
`the_fiscal_year_stays_under_closed_periods`); `ledger-cli`
`tests/spec_kept.rs`; Lean `Step.record_changes_no_figure`,
`Reachable.records`; fuzz `fuzz_settings_replay`; bookspec example
`ledger-a-setting-is-a-fact-in-the-books` (the option picked: the owner's
fiscal-year change is journaled by `Store::change_settings` with who decided
it and when, read back by the setting query, and the trial balance before
and after is the same), run by `every_bookspec_example_holds` and
`the_runner_reports_every_claimed_rule_green`.

### Product rules

**WB-ledger-5** A company's settings, its conventions (how it posts each kind of transaction, its policies and close steps), the decisions its owners made, what is open, and approvals of draft entries are kept in the journal like every other fact (journal format 2: `settings.set`, `convention.set`, `convention.retired`, `decision.recorded`, `open_item.raised`, `open_item.resolved`, `entry.approved`). A settings file is only text to import from or export to; no server reads one (D-44).
*Enforced:* the format gate (format 2 declared before the first such line); `ledger_config::codec` (`import`, `export`).
*Checked:* `ledger-config` `tests/codec_props.rs` (`exports_round_trip`) and codec unit tests; `ledger-store` `tests/settings.rs` (`books_written_under_p1_open_to_their_record`, fixture `old-books/p1_record`).

**WB-ledger-6** No setting, convention id, convention, decision, open item or approval holds a number that reads as a full TIN (nine digits standing alone, bare or printed 3-2-4 or 2-7 with a dash or a space), except the company's own EIN, shown masked everywhere but reports, packages and the viewer, and a D-U-N-S number in its own key.
**Enforced:** `ledger_core::settings::check` and the conventions checks
(`check_new`, `check_decision`, `check_open_item`).
**Checked:** `ledger-core` settings and conventions unit tests;
`ledger-store` `tests/settings.rs` (`settings_replay_is_pure`).

**WB-ledger-7** Books keep their own settings, and only `books.set_setting` changes them; old books bring their settings file in once, whole, as they are adopted (`wanderbooks books adopt`), and nothing reads one after.
*Enforced:* `Store::adopt_settings`; `ledger-mcp` adoption (`local.rs` `settings_to_move`: a settings file, whole, through `codec::import`).
*Checked:* with B-29 above.

**WB-ledger-8** A setting key or convention kind this version doesn't know is a newer version's: the books are "newer", never "damaged" (B-27). Replay takes any value of the right shape (B-28).
*Enforced:* with B-29 above (`shape`, the settings and conventions checks).
*Checked:* `ledger-store` `tests/settings.rs` (`a_newer_key_or_kind_is_newer`).

**WB-ledger-9** A posting template's lines split the amount exactly on each side, odd cents by largest remainder (B-13's rule); a slot (`{expense}`) takes the account each draft names.
*Enforced:* `template_lines`; `Store::draft_from_template` (rehearsed whole).
*Checked:* with B-29 above (the settings and conventions unit tests).

**WB-ledger-10** Approval rules block anything only when the company enforces them (`policy.approval_rules = enforce`; off by default). Then a draft a rule covers posts only once its approver approved it as it stands (its fingerprint: date, description, every line and class), and an operation that would post such an entry at once (an invoice, a bill, a payment, payroll, a remittance, a disposal) is refused: it is drafted instead. Reversals, closes, depreciation and amortization follow from entries already posted and are exempt. A rule names accounts that take postings, and an account a rule names can't be renumbered until the rule is revised.
*Enforced:* `Ledger::approvals_missing`; `Store::post_entry` and `approval_gate` (`ApprovalRequired`, `ApprovalNeedsDraft`), `check_renumber_account`; `ledger-mcp` `books.post_entry` (`approval_required`).
*Checked:* `ledger-store` `tests/settings.rs` (`approval_rules_off_by_default`, `approvals_cover_every_post`); `ledger-core` `tests/conventions_refusals.rs` (`approvals_are_missing_only_under_enforcement_and_fail_closed`, `the_fingerprint_notices_parties_shares_and_reasons`, `an_approval_is_refused_for_a_posted_or_unknown_entry_and_bad_text`, `an_approval_rule_needs_an_approver_and_a_limit`), `tests/ledger_edits.rs` (`an_account_is_renumbered_only_where_the_books_can_stand_it`).

## B-31 Renaming an account changes its label, nothing else

**bookspec:** `BS-ledger-10`

**Claims:** yes

**Pick (BS-ledger-10):** `label-only`, the owner, 2026-10-20.

**Pick:** `label-only`: CHOICE (the owner, `plan/served.md` 4.9 and P3 "Settled in design", 2026-10-20: "a new account named like a sibling (ignoring capitals), or holding a nine-digit number, is refused"). Posted is forever (B-8) stands behind it, and Rev. Proc. 98-25 §5.01 (`sources/irs-records.md`) for keeping the old name. The nine-digit refusal is B-26's: every report prints a name. The 200-character bound is `account::MAX_ACCOUNT_NAME`, the bound of the company's own name.

**Enforced:** `Ledger::rename_account` / `check_rename_account`,
`check_account_name` (also in `add_account`), `account::MAX_ACCOUNT_NAME`,
`check_rename_reason`,
`replay_rename_account` (only that the account exists),
`ChartOfAccounts::set_name`, `Ledger::add_control_account`. The same name
rules hold for a new account (`books.add_account`, `books.import_chart_csv`),
which is journaled with its name trimmed too. **Checked:** `ledger-core`
`tests/spec_ledger.rs` (`rename_account_*`), `tests/model.rs`
(`Cmd::Rename`); `ledger-store` `tests/store.rs`
(`rename_account_changes_only_the_name_defect_15`,
`prop_rename_changes_no_figure`, `a_journaled_rename_replays_past_the_command_rules`),
`tests/format.rs`; `ledger-cli` `tests/mcp_journeys.rs`
(`rename_account_end_to_end_defect_15`), `tests/hostile_args.rs`;
`ledger-core` `tests/spec_ledger.rs`
(`a_new_name_is_one_trimmed_line_of_bounded_length`), `ledger-store`
`tests/store.rs` (`a_new_accounts_name_is_journaled_trimmed`); Lean
`Step.record_changes_no_figure` (a name is outside the model: `docs/verification.md` law 15);
bookspec `examples/rename-changes-no-amount.toml`.

### Product rules

**WB-ledger-11** The journal keeps the old name beside the new one (`account.renamed {account_id, code, name, previous, reason}`), so a report printed under the old name can be tied to the new. Entries name accounts by id, so a rename changes no hash. Approval rules name accounts by code or id, so they do not block a rename. Existing charts are untouched: the rules are the commands', never replay's (B-28).
*Enforced:* `Store::rename_account` (journal feature `account_renamed`, which the books declare before their first rename, B-27); `books.rename_account` (scope `books:post`, like renumbering).
*Checked:* with B-31 above.

**WB-ledger-12** A rename can still change what the books pick by name (no amount): with `books.cash_accounts` unset, the cash accounts are the asset accounts whose names say they hold cash, and, while the books record no business-card payment method, the card accounts are the liabilities named "card" but not "gift" (`ledger_core::parties::card_accounts`, B-35 item 5); the result says when the new name moves the account into or out of either. It also lists conventions whose text names the old name. QuickBooks matches accounts by name, so a CPA who imported an earlier IIF is told.
*Enforced:* `ledger_core::parties::card_accounts`; `books.rename_account`.
*Checked:* with B-31 above.

## B-32 Every revenue and expense account maps to a line of the return

**bookspec:** `BS-ledger-11`

**Pick (BS-ledger-20):** `warn-by-default`, the owner, 2026-10-20

**Pick (BS-ledger-21):** `round-once-on-the-total`, the owner, 2026-11-07, D-67

**Claims:** yes

**Pick:** warn versus refuse: CHOICE (Pub. 583: "the law does not require any specific kind of records"; the owner, 2026-10-20: warn by default, header accounts exempt, the first company set to refuse at the P3 cutover). A category's deductible amount rounds once, on the category total: CHOICE (the owner, 2026-11-07, D-67). The arithmetic is `kernel::deductible_amount`, so Kani proves the one rounding and the bridge carries it to `Decimal`.

**Enforced:** `kernel::deductible_amount` (`tax::tax_summary`'s one
rounding; the bookspec `tax_summary` query answers its lines by category).
**Checked:** `ledger-core`
`tests/tax.rs` (the rounding: `a_category_rounds_its_deductible_amount_once_not_per_account`,
`a_category_rounds_once_across_different_percents`); Kani
`deductible_amount_rounds_once`, `deductible_amount_refuses_overflow`;
bridge `deductible_amount_agrees`; Lean `bookspec/lean/Bookspec/Deductible.lean`
(`Deductible.near`, `Deductible.tie_away_pos`, `Deductible.tie_away_neg`, `Deductible.sign`, `Deductible.neg`, `Deductible.of_sum`,
`Deductible.per_account_differs`, `Deductible.scale_books`, `Deductible.scale_percents`: the one rounding, at any
number of accounts and any sizes); bookspec `examples/tax-category-required.toml`
and the two BS-ledger-21 examples
`ledger-a-categorys-deductible-amount-rounds-once` (the option picked:
two accounts of 1.01 at 50% give 1.01, the exact sum rounded once) and
`ledger-a-categorys-deductible-amount-rounds-per-account` (the other option:
0.51 + 0.51 = 1.02), whose `tax_summary` query is `tax::tax_summary`, run by
`every_bookspec_example_holds` and
`the_runner_reports_every_claimed_rule_green`.

### Product rules

**WB-ledger-13** A tax category is the account's link to the return (`ledger_core::tax`, `sources/irs-tax-lines.md`). Needs attention and the digest (`uncategorized_accounts`) list every account that needs a category and has none, with activity or not. `policy.require_tax_category` is `warn` (unset) or `refuse`. Under `warn`, `books.add_account` and `books.import_chart_csv` create the account and their results say so. Under `refuse`, they refuse it (the file whole), naming the account and where the categories are listed; both take the category at creation, so the refusal is always met in one call; and `books.set_tax_category` won't clear a category an account needs. Turning the policy to `refuse` changes no existing account: it applies to new commands (B-28). Needs attention keeps listing the gaps.
*Enforced:* `tax::needs_category`, `account_needs_category`, `uncategorized_accounts`, `CATEGORIES`; `Ledger::check_tax_category_required`, `check_tax_category_kept` (commands only, never `set_account_tax`, which replay calls); `settings::TaxCategoryPolicy`, `Settings::require_tax_category`; `Store::add_account_with_tax`, `add_account`, `set_account_tax` (journal feature `require_tax_category`, which the books declare before their first `settings.set` naming the key, B-27); `ledger-mcp` `books.add_account`, `books.import_chart_csv`, `books.digest`; `ledger-gui` Needs attention (Info under warn, Todo under refuse).
*Checked:* `ledger-store` `tests/store.rs` (`prop_tax_policy_refuses_exactly`, `a_journaled_clear_replays_under_refuse`); `ledger-cli` `tests/spec_tax_policy.rs`; `ledger-gui` `tests/view_model.rs` (`needs_attention_flags_every_uncategorized_pl_account_defect_8`).

**WB-ledger-14** The accounts invoices, bills and payroll create for a control role (Revenue 4000, General Expenses 6999, Salary & Wage Expense 6100, Payroll Tax Expense 6110) are never refused, so an invoice is never refused over a chart label: their results warn, and Needs attention lists them. The policy is "no uncategorized account made through the chart tools", not "never any". Every account a tool can require has a category that fits it on every return: the gain or loss account of a disposal (`form_4797_gain_loss`, Form 4797), tax-exempt income (`tax_exempt_income`, Schedule K), and each control role's. A gain or loss on business property is figured on Form 4797 from the tax basis, and its split (ordinary gain, net section 1231 gain) decides the line, never Schedule C: the tax summary lists it apart (`TaxSummary::form_4797`), at its book amount, with no tax amount and in no total, on every return.
*Enforced:* `TaxSummary::form_4797`; `tax::CATEGORIES`, with WB-ledger-13's list.
*Checked:* with WB-ledger-13's tests.

## B-44 Evidence is read from the bytes, never trusted over them; identical files are kept once

**retired bookspec:** `BS-ledger-12`

**Claims:** yes (wholly a product rule: BS-ledger-12 is retired, D-71)

**Pick:** none (CONVENTION; Rev. Proc. 98-25 and Pub. 583 for the audit trail).

**Enforced:** the product rule below, `WB-ledger-15` (its *Enforced:*
paragraph names the import, store and attachment functions).
**Checked:** the product rule below, `WB-ledger-15` (its *Checked:*
paragraph names the import, store and CLI tests).

### Product rules

**WB-ledger-15** Evidence is read from the bytes, never trusted over them; identical files are kept once.

```text
GOAL: a receipt's text and the facts read off it are derived records the
      books can re-make; the file's bytes stay the authority; the same
      bytes are one file wherever they are evidence; and a payment says
      what it settles instead of the books guessing.
SOURCE: Rev. Proc. 98-25 §5.01 (machine-sensible records with an audit
        trail: the relationship between the records and the books must be
        demonstrable); IRS Pub. 583 (supporting documents are kept with
        the entries they support); plan/served.md P6 (defects 4 and 10);
        IZ-18 (no FIFO guess);
        CONVENTION (the extractor's version is part of its record; a
        failed read is recorded, not hidden; a duplicate is withdrawn, not
        deleted).
INVARIANTS:
  1. An attachment's text is a record (`attachment.text_extracted`) with
     the text's content-addressed blob, the extractor and its version, the
     page count and `ok`, `none` or `failed`. One record per attachment,
     extractor and version: an upgrade adds a record and NEVER replaces
     one, and the bytes are never re-derived by `verify` (its hash is
     checked).
  2. Extraction is derived and journaled, never authoritative: a report,
     a posting or an audit reads the file's bytes when the text matters,
     and a failed extraction is a record that a human must read the
     pages (D-15: no OCR).
  3. Identical bytes are one file: filing bytes already stored (and not
     withdrawn) returns the existing attachment, linked to the new
     subject. Consolidating legacy duplicates keeps the oldest, links it
     to every subject the others had, and withdraws the others with
     `duplicate_of`; nothing is deleted and no posted entry the receipt
     policy covers is left without evidence.
  4. An annotation (`attachment.annotated`) records what a human read off
     a file and changes no amount, no file and no posting; its fields are
     a closed list. At least one field or the note is present.
  5. `entry.settles` names the entries a payment pays back, every named
     entry existing, none itself, none twice. "Reimbursed" is derived from
     it alone: where no settlement is recorded, Needs attention asks; the
     books never guess (IZ-18).
  6. Every refusal leaves the journal unchanged; NEVER panics.
SCENARIOS:
  1. A text PDF filed: one extraction record, its text a blob, `ok`.
  2. The same PDF filed again: one attachment, linked to the second
     subject, `deduplicated: true`.
  3. Two legacy records with one blob, the oldest withdrawn, the youngest
     an entry's only receipt: consolidation is refused whole.
  4. The same, both live: the oldest is linked to the other's subjects,
     the other is unlinked and withdrawn with `duplicate_of`.
  5. A receipt annotated 7 on two different files: the next engine number
     is 8 (numbers run from the max; a human's duplicate is not renumbered).
  6. An amount 20.00 paid by a member, reimbursed: the reimbursement's
     `entry.settles` names the expense; no Needs-attention item remains.
  7. The same with no settlement: Needs attention asks, once.
```

*Enforced:* `ledger_import::extract` (`pdf_text`, `pdf_lines`,
`page_png`, `xlsx_text`, `text_of`, `extract`; caps 25 MiB, 30 pages,
2 MiB of text; a reader panic is caught), `ledger_core::evidence::{Annotation,
AnnotationFields}`, `Ledger::{annotate_attachment, next_receipt_number,
entry_settles, settles_entry, set_entry_settles}`,
`Store::{extract_attachment, annotate_attachment, set_entry_settles,
withdraw_attachment_duplicate}` and `Store::attach`'s dedup; feature
`attachment_text`.
*Checked:* `ledger-import` `tests/extract.rs`
(`extraction_deterministic`, `xlsx_cells_are_decimal_strings`,
`a_pdf_that_crashes_the_reader_is_failed_not_fatal`); `ledger-store`
`tests/attachments.rs` (`extraction_is_journaled_once_per_version`,
`extract_waits_for_attachment_text`, `same_bytes_one_record_many_links`,
`legacy_duplicates_replay_as_written`,
`annotations_replay_and_project`, `receipt_numbers_run_from_the_max`,
`reimbursed_is_derived_from_settles_only`); `ledger-cli`
`tests/evidence.rs` (`get_receipt_default_has_text_and_no_bytes`,
`page_image_is_a_png_of_the_page`,
`consolidation_never_removes_an_entrys_only_evidence`); `ledger-core`
`tests/evidence_gate.rs` (`annotation_fields_are_trimmed_and_blank_ones_dropped`,
`an_annotation_field_alone_is_a_record`,
`an_annotation_refuses_a_zero_number_a_negative_amount_and_a_date_off_the_calendar`,
`an_annotation_needs_a_field_or_a_note`), `tests/ledger_edits.rs`
(`annotations_replace_by_file_and_receipt_numbers_run_from_the_max`,
`a_settlement_of_entries_names_existing_distinct_entries_only`); fuzz targets
`fuzz_extract_pdf` and `fuzz_extract_xlsx`.

## B-45 An audit is deterministic over the books; an attestation is a claim by a named client

**retired bookspec:** `BS-ledger-13`

**Claims:** yes (wholly a product rule: BS-ledger-13 is retired, D-71)

**Pick:** CHOICE D-9 (owner-adopted recommendation, 2026-10-16): the attestation bar, and the CPA package waits for a counted audit unless given a reason. Otherwise CONVENTION (the SOURCE block below; Pub. 583 and Rev. Proc. 98-25 for the audit trail).

**Enforced:** the product rule below, `WB-ledger-16` (its *Enforced:*
paragraph names the audit and store functions).
**Checked:** the product rule below, `WB-ledger-16` (its *Checked:*
paragraph names the core, store, import and CLI tests).

### Product rules

**WB-ledger-16** An audit is deterministic over the books; an attestation is a claim by a named client.

```text
GOAL: the books can be audited by a programme that gives the same
      findings for the same books, and a claim that someone checked rests
      on the books as they were when the audit ran, says how it was made,
      and is weighed against the owner's bar when asked.
SOURCE: IRS Pub. 583 (records that support the entries); Rev. Proc. 98-25
        §5.01 (the relationship between the records and the books must be
        demonstrable: an audit trail); CHOICE D-9 (the bar: reasoning
        effort at least high; for a year, adversarial verification by at
        least two independent agents; no model allow-list; the CPA package
        refuses without a counted audit unless given a reason); CHOICE
        D-53 (an audit is a run, not a job); CONVENTION (the figure head,
        below; a client's method is its own claim).
INVARIANTS:
  1. An audit is a pure function of the journal's figures: its findings
     (`ledger_core::audit::statements`, the store's integrity checks) are
     a list in a fixed order, and the same books, day, scope and
     programme give the same findings, the same trial balance hash and
     the same workpaper bytes. A check that cannot be made is itself a
     finding, never a silent pass.
  2. The figure head is the seq of the last journal line that bears on a
     figure. Audit runs, attestations, filings of evidence (receipts,
     documents, statements, reconciliations, extraction, annotations,
     links), decisions and open items, and the format declaration do not
     move it; an entry, a draft, a document, a close, a schedule, an
     asset, a settings change or a role change does. A run is idempotent
     on (through, head, scope, programme): asking again returns it and
     journals nothing.
  3. A period is attested only on a run at the current head whose day is
     on or after the period's end.
  4. An attestation is a claim by the token that made it (`by`); its
     method (model, reasoning effort, adversarial, verifier agents) is
     the client's own, recorded and never verified. A receipt attestation
     whose amount or date differs from the entry's is recorded with
     `disagrees` and never counts.
  5. `counted` is derived against `policy.attestation_bar` as it stands
     when asked and never stored: a bar change changes what counts, past
     claims too. Every attestation needs the reasoning effort; one on a
     year-level run (annual, cpa_handoff, full) needs the adversarial
     check and the verifier count too.
  6. The CPA package, on books that declared `audits` and under
     `policy.cpa_package_requires_audit = refuse` (unset: refuse once the
     books hold an audit run, warn before), needs a run of scope
     cpa_handoff or full at the current head with a period attested on it
     that counts; otherwise it is refused, unless a reason is given,
     which is journaled as a decision.
  7. Nothing here moves a figure. Every refusal leaves the journal
     unchanged; every write waits for the feature `audits`.
  8. `close_split_drifts` holds a members close to BS-partner-1
     statement 5: once the count has had a close under the cumulative
     rule that posted an entry (its true-up, statement 6; the close
     journals its rule and whether it posted), a member's running total
     MUST be under one minor unit from its exact share, so a drift of a
     unit or more is a finding. The unit is the allocation's own
     (`Currency::allocation_unit`, on `allocation_places`: a whole yen
     for JPY, the finest a line carries for a code with no minor unit),
     never a cent taken separately. Before that true-up the count may
     still carry the rounding of closes made under the older rules, so
     only a drift beyond B(n) is a finding, where B(n) = max(one unit,
     (n-1) half-units) for n members: a unit with three or fewer
     members, 1.5 units with four, 2 units with five. B(n) is DERIVED:
     BS-partner-1 statement 6 states the per-member facts it comes from
     (a last-absorbs close rounded each member but the last to within
     half a unit, and the last absorbed up to half a unit for each of
     the others; under a unit for each run break and each per-close
     close), not the bound itself. Closes under the older rules are not
     read.
SCENARIOS:
  1. The same books audited twice through one day: one run, one
     workpaper; the second call files and journals nothing.
  2. An entry posted after the audit: the period cannot be attested until
     the audit is run again.
  3. A receipt read as 41.00 against a 40.00 entry: recorded, `disagrees`,
     not counted, however hard the checker says it thought.
  4. The bar lowered afterwards: earlier attestations that now meet it
     count.
  5. A CPA package with no counted audit: refused (`audit_required`);
     with a reason, made, and the reason is a decision in the journal.
  6. Books that never declared `audits`: the package is as it was; the
     audit run and the attestations are refused with
     `format_upgrade_pending`.
```

*Enforced:* `ledger_core::audit` (`statements`, `Finding`, `CODES`,
`AttestationBar::counts`, `Scope`, `Method`, `check_split`;
`Ledger::member_close_trued_up`), `ledger_store::audit`
(`Store::figure_head`, `integrity_findings`, `record_audit_run`,
`attest_receipt`, `review_entry`, `attest_period`, `run_for_period`,
`attestations`, `PROGRAM_VERSION`), `ledger_import::workpaper_pdf`,
ledger-mcp `tools/audit.rs` and the gate in `books.export_cpa_package`,
journal feature `audits`.
*Checked:* `ledger-core` `tests/audit.rs`
(`audit_detects_each_seeded_defect`, `below_bar_not_counted`,
`amount_mismatch_never_counts`), `tests/audit_branches.rs`
(`the_cash_flow_check_says_what_it_found`,
`control_accounts_report_differences_unattached_lines_and_overpaid_documents`,
`a_close_split_drifts_by_the_regime_it_was_made_under`,
`a_jpy_close_is_checked_at_the_allocations_unit`,
`a_legacy_last_absorbs_close_drifts_by_member_count_before_its_true_up`,
`a_count_is_trued_up_by_the_first_cumulative_close_that_posts`,
`a_reversal_without_its_own_receipt_is_info_and_only_when_the_policy_would_cover_it`,
`the_bar_setting_parses_what_it_names_and_refuses_the_rest`,
`the_bar_counts_at_its_edges_and_only_asks_for_what_it_sets`); `ledger-store` `tests/audit.rs`
(`audit_writes_wait_for_audits`, `a_run_is_idempotent_on_the_figure_head`,
`stale_head_period_attestation_refused`, `integrity_detects_each_seeded_defect`,
`books_the_version_before_p8_wrote_wait_for_the_upgrade`); `ledger-import`
`workpaper_pdf_is_deterministic`; `ledger-cli` `tests/audit.rs`
(`audit_is_deterministic`, `cpa_package_refuses_without_a_counted_audit_unless_a_reason_is_given`,
`audits_wait_for_the_upgrade`).

## Reconciliations: which accounts the monthly close reconciles, and when each is due

**bookspec:** none

**Claims:** yes (wholly a product rule: no bookspec rule states it)

**Pick:** CONVENTION, `docs/parties-and-attachments.md` "Reconciliations" (Needs attention: an account with activity not reconciled through last month's end; bank and card statements close monthly). No authority states it, so it is the product's own.

### Product rules

**WB-ledger-17** The accounts the monthly close reconciles, and when each is due, are one rule (`ledger_store::reconcilable`), read by the digest's `unreconciled_accounts` and the viewer's Needs attention alike.

```text
GOAL: the monthly close's reconciliation step can't be forgotten, and
      the digest and the viewer never disagree on what is due.
SOURCE: CONVENTION (bank and card statements close monthly);
        docs/parties-and-attachments.md "Reconciliations".
INVARIANTS:
  1. The accounts reconciled are the cash accounts, then the card
     accounts, then every other liability with a recorded statement, each
     once. The cash accounts are `books.cash_accounts` resolved by code, in
     the order given (a code the chart lacks is dropped, and the names are
     not then consulted); when none is set, the asset accounts that are
     not placeholders and whose names say cash ("cash", "bank",
     "checking", "savings", "money market"), in ascending account id. The
     card accounts are `ledger_core::parties::card_accounts`, in ascending
     account id. The liabilities with a statement follow in the order the
     statements are given (the store's: period end, then file). A card is
     expected before its first statement; a liability not named a card
     joins once its first statement is recorded; an asset with a statement
     and no cash name never does.
  2. An account is due through the last day of the month before today's
     when it has a posted line dated on or before that day and no
     reconciliation not voided reaches it (is dated on or after that
     day); otherwise it is not due. A draft line, activity only this
     month, and an account with no lines are not due; a voided
     reconciliation reaches nothing.
SCENARIOS:
  1. Cash named "Checking" and a card named "Visa Card", an "Amex" with
     no statement: the checking and the card are reconciled, the Amex is
     not; once the Amex has a statement it joins, after the card.
  2. Today is March 10th, the checking has January activity and a
     reconciliation through January 31st: it is due through February
     28th; one through February 28th clears it; voiding that one makes it
     due again.
```

**Enforced:** `reconcilable_accounts`, `cash_accounts`, `card_accounts`, `last_month_end`, `reconciled_through`, `reconciliation_due` (`ledger_store::reconcilable`).
**Checked:** `ledger-store` `tests/reconcilable.rs` (`cash_accounts_are_the_assets_whose_names_say_cash`, `cash_accounts_by_name_come_in_ascending_account_id`, `configured_cash_accounts_win_by_code`, `card_accounts_follow_the_recorded_method_else_the_name`, `reconcilable_accounts_are_cash_then_cards_then_liabilities_with_statements`, `last_month_end_is_the_last_day_of_the_month_before`, `reconciled_through_is_the_newest_standing_reconciliation`, `an_account_is_due_until_a_reconciliation_reaches_last_months_end`, `only_posted_activity_through_last_months_end_makes_an_account_due`).
