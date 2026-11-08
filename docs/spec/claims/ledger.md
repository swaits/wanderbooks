# Claims: ledger

Wanderbooks' claims for bookspec area **ledger**: what each rule requires, where the engine enforces it, and which tests check it. Being reduced to the engine's pick and its evidence as P14 unit S-D lands; the accounting text moves to bookspec's `spec/ledger.md`.

Rules: B-3, B-4, B-5, B-8, B-9, B-10, B-20, B-23, B-29, B-31, B-32, B-44, B-45.

---

## B-3 One money through the ledger; foreign amounts at the transaction-date rate

**bookspec:** `BS-ledger-1`

**Status:** AUTHORITY

- Pacioli ch. 36: "the money that you reckon by at the beginning should be
  the same all through the Ledger."
- ASC 830-20-30-1 (cited): "each asset, liability, revenue, expense, gain,
  or loss arising from the transaction shall be measured initially in the
  functional currency of the recording entity by use of the exchange rate
  in effect at that date."

A foreign line is booked in the base currency's smallest unit (ISO 4217
minor units, `sources/iso-4217.md`), half away from zero; a ledger in one
money holds no fractions of its smallest unit, and its cash can then
match a bank statement: CHOICE (the owner, 2026-10-13), after the GnuCash
cross-check found wanderbooks kept $108.34567 where GnuCash booked
$108.35. Lines booked before then replay as booked.

**Enforced:** `Ledger::add_line` (base currency only, no fraction of the
minor unit), `Ledger::add_line_fx` (base amount = foreign amount × rate,
rounded to the minor unit; original kept), `Ledger::post_entry` (every
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
`foreign_currency_line_is_rejected`, `fx_overflow_is_an_error`; the
GnuCash oracle.

## B-4 Five elements, closed list

**bookspec:** `BS-ledger-2`

**Status:** AUTHORITY (definitions); the five-type chart is CONVENTION

FASB Concepts Statement No. 8, Ch. 4 (2021), which replaced Concepts
Statement 6 (cited): E16 "An asset is a present right of an entity to an
economic benefit." E37 "A liability is a present obligation of an entity to
transfer an economic benefit." E61 equity is "the residual interest in the
assets of an entity that remains after deducting its liabilities." E80
revenues "are inflows or other enhancements of assets … or settlements of
its liabilities … from delivering or producing goods, rendering services,
or carrying out other activities." E81 expenses "are outflows or other
using up of assets … or incurrences of its liabilities …".

**Enforced:** `AccountType` (five variants). A child account has its
parent's type: CHOICE (the owner, 2026-10-13), `Ledger::add_account`
(`ParentTypeMismatch`). **Checked:** `a_child_account_has_its_parents_type`.

## B-5 Normal balances

**bookspec:** `BS-ledger-3`

**Status:** CONVENTION

No standard defines normal balances. OpenStax, *Principles of Accounting,
Vol. 1* §3.1 (cited; "Access for free at openstax.org"): "As assets and
expenses increase on the debit side, their normal balance is a debit…
Since liabilities, equity (such as common stock), and revenues increase
with a credit, their 'normal' balance is a credit." Pacioli ch. 12: capital
"must always be placed as creditor … and the cash always debtor."

**Enforced:** `AccountType::normal_side`, `kernel::normal_balance`.
**Checked:** Kani `normal_balance_is_signed`, `normal_side_increases`; Lean
`balance_after_line`.

## B-8 Posted is forever: the book is written only forward

**bookspec:** `BS-ledger-4`

**Status:** AUTHORITY

- Pacioli ch. 36: "You should not write backward, but always forward —
  that is, go forward as the days go, which never come back. If you do
  otherwise, the book would be untrue."
- German Commercial Code (HGB) §239(3) (cited; official English
  translation): "An entry or a documented record may not be modified such
  that it is no longer possible to establish its original content."
- Rev. Proc. 98-25 (`sources/irs-records.md`): machine-sensible records
  must keep the audit trail.

**Enforced:** posted entries immutable; the journal is append-only and
hash-chained. **Checked:** Lean `Step.extends`, `Steps.extends`; property
`prop_posted_entries_immutable`; store `journal_tamper_is_detected`.

## B-9 A mistake is corrected by an opposite entry, never erased

**bookspec:** `BS-ledger-5`

**Status:** AUTHORITY

Pacioli ch. 31 (the *storno*): "you shall make another entry opposite this
one in the credit for the same amount … After you have made this
correction it is just as if you had written nothing in the debit column."
Ch. 36: "Mark with a cross or with an "H" that special entry, and then make
an entry on the opposite side under the same account."

**Enforced:** `Ledger::reverse_entry` (same lines, sides flipped, linked by
`reversal_of`). **Checked:** Kani `reversal_mirrors`, `reversal_cancels`;
Lean `accepted_reversal_iff`, `movement_with_reversal`.

## B-10 An entry is reversed at most once; a closing entry never

**bookspec:** `BS-ledger-6`

**Status:** CHOICE (the owner, 2026-10-13)

Follows from B-9 (a correction corrects once) and B-11 (a closed period
stays closed). **Enforced:** `Ledger::check_reversal`. **Checked:**
`an_entry_is_reversed_at_most_once`, `a_closing_entry_is_never_reversed`.

## B-20 Every transaction has supporting documents; records are kept

**bookspec:** `BS-ledger-7`

**Status:** AUTHORITY (`sources/irs-records.md`)

IRS Pub. 583: "In addition, you must keep supporting documents"; they
"support the entries in your books and on your tax return"; keep records
"until the period of limitations for that return runs out" (3 years
generally; 7 for bad debts).

**Enforced:** the MCP receipt policy (not the engine; R-13).

## B-23 Control roles: one account each, of the right kind, moved only when empty

**bookspec:** `BS-ledger-8`

**Status:** CONVENTION (a control account agrees with the records it
controls); CHOICES (the owner, 2026-10-13 and 2026-10-15)

Invoices, bills, payroll, and closes post to accounts the engine picks by
role: receivables, revenue, payables, general expense, retained earnings,
salary and payroll tax expense, and five payroll withholdings. A role uses
its default code (1200, 4000, 2000, 6999, 3999, 6100, 6110, 2110–2150)
until the human assigns it one of the chart's own accounts; the assignment
is journaled, so replay never depends on a setting outside the journal
(CHOICE, 2026-10-15). A role's account is postable and of the role's
type (B-4), and the engine refuses one that isn't rather than renumber
the chart (CHOICE, 2026-10-13). No account fills two roles. A
receivables, payables, withholding, or retained earnings account must keep
agreeing with what it controls (open invoices, open bills, taxes withheld,
closed earnings): such a role moves only when its account holds no
balance. For a withholding or retained earnings role the bookkeeper
first moves the balance with an entry, which the journal shows. Revenue
and expense roles move at any time; earlier postings stay where they
are. Receivables and payables are different (B-38, 2026-10-23): their
documents read their figures from their lines, so their control accounts
take no line by hand and equal their documents by construction. Their
balance moves with its documents only (plan P4b S7); until then a
receivables or payables role whose control account holds a balance stays
where it is, and the refusal says so (it names no hand entry, which the
control guard would refuse). Such a role also never moves onto an
account holding a line that is no document's, and the account it leaves
stays guarded while it holds its documents' lines. The account at a
role's default code is the role's control account only once a document
posts there (or the role is assigned to it): before that, a chart's own
1200 Equipment or a card at 2000 is the chart's.

**Enforced:** `Ledger::check_controls`, `Ledger::assign_role` /
`check_assign_role`, `Ledger::check_renumber_account` (an assigned role
frees its code); the control guard and `Ledger::control_check` (B-38).
**Checked:** `ledger-core` `tests/spec_roles.rs`, `tests/spec_documents.rs`;
`ledger-store` replay of `role.assigned`.

## B-29 The books keep how they are kept

**bookspec:** `BS-ledger-9`

**SOURCE:** CHOICE (plan/served.md I-3, owner decisions of 2026-10-16:
"the journal holds all books state"; light governance: decisions are
recorded with who made them and when, not as paperwork); IRS Pub. 538 and
the rules of B-1 to B-28 for what each setting means.

A company's settings, its conventions (how it posts each kind of
transaction, its policies and close steps), the decisions its owners made,
what is open, and approvals of draft entries are facts about the books,
kept in the journal like every other fact, each with who decided it and
when (journal format 2: `settings.set`, `convention.set`,
`convention.retired`, `decision.recorded`, `open_item.raised`,
`open_item.resolved`, `entry.approved`). A settings file is only text to
import from or export to; no server reads one (D-44).

- A setting is one of the known keys, checked by its key and kept in one
  spelling; a change is all or nothing, and the settings after it hold
  together. Replay takes any value of the right shape (B-28).
- No setting, convention id, convention, decision, open item or approval
  holds a number that reads as a full TIN (nine digits standing alone,
  bare or printed 3-2-4 or 2-7 with a dash or a space), except the
  company's own EIN, shown masked everywhere but reports, packages and the
  viewer, and a D-U-N-S number in its own key.
- A record of how the books are kept changes no entry, draft, period or
  account (proved for the model: `Step.record_changes_no_figure`; checked
  on the engine after every record in `settings_replay_is_pure`). Settings
  that say how the same entries are presented or later allocated (the
  fiscal year, the cash flow roles, the members) present them by the
  settings in force; the fiscal year can't move once a period is closed
  under it (a changed tax year needs the IRS's consent, Form 1128).
- Books keep their own settings, and only `books.set_setting` changes
  them; old books bring their settings file in once, whole, as they are
  adopted (`wanderbooks books adopt`), and nothing reads one after.
- A setting key or convention kind this version doesn't know is a newer
  version's: the books are "newer", never "damaged" (B-27).
- A posting template's lines split the amount exactly on each side, odd
  cents by largest remainder (B-13's rule); a slot (`{expense}`) takes the
  account each draft names.
- Approval rules block anything only when the company enforces them
  (`policy.approval_rules = enforce`; off by default). Then a draft a rule
  covers posts only once its approver approved it as it stands (its
  fingerprint: date, description, every line and class), and an operation
  that would post such an entry at once (an invoice, a bill, a payment,
  payroll, a remittance, a disposal) is refused: it is drafted instead.
  Reversals, closes, depreciation and amortization follow from entries
  already posted and are exempt. A rule names accounts that take postings,
  and an account a rule names can't be renumbered until the rule is
  revised.

**Enforced:** `ledger_core::settings` (`check`, `Settings::check_changes`,
`check_whole`, `shape`, `apply`), `ledger_core::conventions` (`check_new`,
`check_decision`, `check_open_item`, `split`), `Ledger::check_settings`,
`check_convention`, `approvals_missing`, `template_lines`; `ledger-store`
`Store::change_settings`, `adopt_settings`, `set_convention`,
`draft_from_template` (rehearsed whole), `record_decision`; the format gate
(format 2 declared before the first such line); `ledger_config::codec`
(`import`, `export`); `Store::post_entry` and `approval_gate`
(`ApprovalRequired`, `ApprovalNeedsDraft`), `check_renumber_account`;
`ledger-mcp` adoption (`local.rs` `settings_to_move`: a settings file,
whole, through `codec::import`), `books.post_entry`
(`approval_required`).
**Checked:** `ledger-core` settings and conventions unit tests;
`ledger-store` `tests/settings.rs` (`settings_replay_is_pure`, 256 cases
per check, 2,000 by hand, with posts between records and every record
checked to change no figure or entry; `approval_rules_off_by_default`,
`approvals_cover_every_post`, `the_fiscal_year_stays_under_closed_periods`,
`a_newer_key_or_kind_is_newer`;
`books_written_under_p1_open_to_their_record`, fixture
`old-books/p1_record`); `ledger-config` `tests/codec_props.rs`
(`exports_round_trip`) and codec unit tests; `ledger-cli`
`tests/spec_kept.rs`; Lean `Step.record_changes_no_figure`,
`Reachable.records`; fuzz `fuzz_settings_replay`.

## B-31 Renaming an account changes its label, nothing else

**bookspec:** `BS-ledger-10`

**Status:** CHOICE (the owner, `plan/served.md` 4.9 and P3 "Settled in
design", 2026-10-20: "a new account named like a sibling (ignoring
capitals), or holding a nine-digit number, is refused"); B-8 (posted is
forever); AUTHORITY for keeping the old name (Rev. Proc. 98-25 §5.01,
`sources/irs-records.md`: "the relationship (i.e., audit trail): (a)
between the total of the amounts in the taxpayer's machine-sensible
records by account and the account totals in the taxpayer's books");
Intuit IIF (`sources/intuit-iif.md`: "Accounts are identified by NAME")
for refusing a sibling's name.

Entries name accounts by id, so a rename changes no entry, line, hash,
balance or statement amount: every report, past periods included, prints
the new name. The journal keeps the old one (`account.renamed {account_id,
code, name, previous, reason}`), so a report printed under the old name
can be tied to the new. A rename is refused when the account is unknown,
the name is blank, the name (trimmed) is the one it has, another account
under the same parent (top-level accounts are siblings too) has it
ignoring capitals and surrounding space (QuickBooks would merge the two),
it holds a run of nine digits (a full SSN, EIN or account number, B-26:
every report prints a name), or it is not one line of at most 200
characters (`account::MAX_ACCOUNT_NAME`, the bound of the company's own
name: no line break, tab or other control character, which no report, CSV
or IIF line can hold). The name is kept trimmed. The same name rules hold
for a new account (`books.add_account`, `books.import_chart_csv`), which
is journaled with its name trimmed too; accounts the engine
creates for a control role keep their fixed names and are never refused
for one. Approval rules, roles, settings and closed periods name accounts
by code or id, and a name is no posting, so none of them blocks a rename.
Existing charts are untouched: the rules are the commands', never replay's
(B-28).

A rename can still change what the books pick by name (no amount): with
`books.cash_accounts` unset, the cash accounts are the asset accounts
whose names say they hold cash, and, while the books record no
business-card payment method, the card accounts are the liabilities
named "card" but not "gift" (`ledger_core::parties::card_accounts`, B-35
item 5); the result says
when the new name moves the account into or out of either. It also lists
conventions whose text names the old name. QuickBooks matches accounts
by name, so a CPA who imported an earlier IIF is told.

**Enforced:** `Ledger::rename_account` / `check_rename_account`,
`check_account_name` (also in `add_account`), `account::MAX_ACCOUNT_NAME`,
`check_rename_reason`,
`replay_rename_account` (only that the account exists),
`ChartOfAccounts::set_name`, `Ledger::add_control_account`;
`Store::rename_account` (journal feature `account_renamed`, which the
books declare before their first rename, B-27); `books.rename_account` (scope
`books:post`, like renumbering). **Checked:** `ledger-core`
`tests/spec_ledger.rs` (`rename_account_*`), `tests/model.rs`
(`Cmd::Rename`); `ledger-store` `tests/store.rs`
(`rename_account_changes_only_the_name_defect_15`,
`prop_rename_changes_no_figure`, `a_journaled_rename_replays_past_the_command_rules`),
`tests/format.rs`; `ledger-cli` `tests/mcp_journeys.rs`
(`rename_account_end_to_end_defect_15`), `tests/hostile_args.rs`;
`ledger-core` `tests/spec_ledger.rs`
(`a_new_name_is_one_trimmed_line_of_bounded_length`), `ledger-store`
`tests/store.rs` (`a_new_accounts_name_is_journaled_trimmed`); Lean
`Step.record` (a name is outside the model: `docs/verification.md` law 15);
bookspec `examples/rename-changes-no-amount.toml`.

## B-32 Every revenue and expense account maps to a line of the return

**bookspec:** `BS-ledger-11`

**Status:** AUTHORITY for why it matters (IRS Pub. 583,
`sources/irs-records.md`: "Your books must show your gross income, as
well as your deductions and credits"; IRS Pub. 538: "you must maintain
records that will enable you to file a correct return"; Rev. Proc. 98-25
§5.01: the relationship "between the total of the amounts in the
taxpayer's machine-sensible records by account and the taxpayer's
return"); CHOICE for warn versus refuse (Pub. 583: "the law does not
require any specific kind of records"; the owner, 2026-10-20: warn by
default, header accounts exempt, the first company set to refuse at the
P3 cutover; and, the owner, 2026-11-07, D-67: a category's deductible amount
rounds once, on the category total).

A tax category is the account's link to the return (`ledger_core::tax`,
`sources/irs-tax-lines.md`). An account needs one when it is a revenue or
expense account and not a header (a header takes no postings, so nothing
of its own reaches the return: no entry posts to one, a replayed one
included, and an account's header flag never changes).

- Needs attention and the digest (`uncategorized_accounts`) list every
  account that needs a category and has none, with activity or not.
- `policy.require_tax_category`: `warn` (unset) or `refuse`. Under `warn`,
  `books.add_account` and `books.import_chart_csv` create the account and
  their results say so. Under `refuse`, they refuse it (the file whole),
  naming the account and where the categories are listed; both take the
  category at creation, so the refusal is always met in one call; and
  `books.set_tax_category` won't clear a category an account needs.
- The accounts invoices, bills and payroll create for a control role
  (Revenue 4000, General Expenses 6999, Salary & Wage Expense 6100,
  Payroll Tax Expense 6110) are never refused, so an invoice is never
  refused over a chart label: their results warn, and Needs attention
  lists them. The policy is "no uncategorized account made through the
  chart tools", not "never any".
- Turning the policy to `refuse` changes no existing account: it applies
  to new commands (B-28). Needs attention keeps listing the gaps.
- CHOICE (owner, 2026-11-07): a category's deductible amount in the tax
  summary is the exact sum of each of its accounts' shares (book ×
  deductible percent ÷ 100), rounded once to the cent, half away from
  zero, following B-21's "round off only the total"; the books stay
  exact (R-2). The arithmetic is `kernel::deductible_amount`, so Kani
  proves the one rounding and the bridge carries it to `Decimal`.
- Every account a tool can require has a category that fits it on every
  return: the gain or loss account of a disposal (`form_4797_gain_loss`,
  Form 4797), tax-exempt income (`tax_exempt_income`, Schedule K), and
  each control role's. A gain or loss on business property is figured on
  Form 4797 from the tax basis, and its split (ordinary gain, net section
  1231 gain) decides the line, never Schedule C: the tax summary lists it
  apart (`TaxSummary::form_4797`), at its book amount, with no tax amount
  and in no total, on every return.

**Enforced:** `tax::needs_category`, `account_needs_category`,
`uncategorized_accounts`, `CATEGORIES`; `kernel::deductible_amount`
(`tax::tax_summary`'s one rounding); `Ledger::check_tax_category_required`,
`check_tax_category_kept` (commands only, never `set_account_tax`, which
replay calls); `settings::TaxCategoryPolicy`, `Settings::require_tax_category`;
`Store::add_account_with_tax`, `add_account`, `set_account_tax`
(journal feature `require_tax_category`, which the books declare before
their first `settings.set` naming the key, B-27); `ledger-mcp` `books.add_account`,
`books.import_chart_csv`, `books.digest`; `ledger-gui` Needs attention
(Info under warn, Todo under refuse). **Checked:** `ledger-core`
`tests/tax.rs` (the rounding: `a_category_rounds_its_deductible_amount_once_not_per_account`,
`a_category_rounds_once_across_different_percents`); Kani
`deductible_amount_rounds_once`, `deductible_amount_refuses_overflow`;
bridge `deductible_amount_agrees`; Lean `verify/lean/Wanderbooks/Deductible.lean`
(`Deductible.near`, `tie_away_pos`, `tie_away_neg`, `sign`, `neg`, `of_sum`,
`per_account_differs`, `scale_books`, `scale_percents`: the one rounding, at any
number of accounts and any sizes); `ledger-store` `tests/store.rs`
(`prop_tax_policy_refuses_exactly`, `a_journaled_clear_replays_under_refuse`);
`ledger-cli` `tests/spec_tax_policy.rs`; `ledger-gui` `tests/view_model.rs`
(`needs_attention_flags_every_uncategorized_pl_account_defect_8`);
bookspec `examples/tax-category-required.toml`.

## B-44 Evidence is read from the bytes, never trusted over them; identical files are kept once

**bookspec:** `BS-ledger-12`

**Status:** CONVENTION (the SOURCE block below; Rev. Proc. 98-25 and Pub. 583 for
the audit trail)

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

**Enforced:** `ledger_import::extract` (`pdf_text`, `pdf_lines`,
`page_png`, `xlsx_text`, `text_of`, `extract`; caps 25 MiB, 30 pages,
2 MiB of text; a reader panic is caught), `ledger_core::evidence::{Annotation,
AnnotationFields}`, `Ledger::{annotate_attachment, next_receipt_number,
entry_settles, settles_entry, set_entry_settles}`,
`Store::{extract_attachment, annotate_attachment, set_entry_settles,
withdraw_attachment_duplicate}` and `Store::attach`'s dedup; feature
`attachment_text`.
**Checked:** `ledger-import` `tests/extract.rs`
(`extraction_deterministic`, `xlsx_cells_are_decimal_strings`,
`a_pdf_that_crashes_the_reader_is_failed_not_fatal`); `ledger-store`
`tests/attachments.rs` (`extraction_is_journaled_once_per_version`,
`extract_waits_for_attachment_text`, `same_bytes_one_record_many_links`,
`legacy_duplicates_replay_as_written`,
`annotations_replay_and_project`, `receipt_numbers_run_from_the_max`,
`reimbursed_is_derived_from_settles_only`); `ledger-cli`
`tests/evidence.rs` (`get_receipt_default_has_text_and_no_bytes`,
`page_image_is_a_png_of_the_page`,
`consolidation_never_removes_an_entrys_only_evidence`); fuzz targets
`fuzz_extract_pdf` and `fuzz_extract_xlsx`.

## B-45 An audit is deterministic over the books; an attestation is a claim by a named client

**bookspec:** `BS-ledger-13`

**Status:** CONVENTION (the SOURCE block below; Pub. 583 and Rev. Proc. 98-25
for the audit trail); CHOICE D-9 (owner-adopted recommendation,
2026-10-16: the attestation bar, and the CPA package waits for a counted
audit unless given a reason).

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

**Enforced:** `ledger_core::audit` (`statements`, `Finding`, `CODES`,
`AttestationBar::counts`, `Scope`, `Method`), `ledger_store::audit`
(`Store::figure_head`, `integrity_findings`, `record_audit_run`,
`attest_receipt`, `review_entry`, `attest_period`, `run_for_period`,
`attestations`, `PROGRAM_VERSION`), `ledger_import::workpaper_pdf`,
ledger-mcp `tools/audit.rs` and the gate in `books.export_cpa_package`,
journal feature `audits`.
**Checked:** `ledger-core` `tests/audit.rs`
(`audit_detects_each_seeded_defect`, `below_bar_not_counted`,
`amount_mismatch_never_counts`); `ledger-store` `tests/audit.rs`
(`audit_writes_wait_for_audits`, `a_run_is_idempotent_on_the_figure_head`,
`stale_head_period_attestation_refused`, `integrity_detects_each_seeded_defect`,
`books_the_version_before_p8_wrote_wait_for_the_upgrade`); `ledger-import`
`workpaper_pdf_is_deterministic`; `ledger-cli` `tests/audit.rs`
(`audit_is_deterministic`, `cpa_package_refuses_without_a_counted_audit_unless_a_reason_is_given`,
`audits_wait_for_the_upgrade`).
