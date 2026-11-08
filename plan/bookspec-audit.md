# bookspec audit (2026-11-08): what it takes to make bookspec THE specification

Read-only audit. Nothing in the code, the proofs or the specs was changed.
Inputs: wanderbooks at `a2ad2ed3` (`docs/spec/rules.md` 3,696 lines, 27 files
in `docs/spec/sources/`, `verify/lean/` 10 files and 118 audited theorems,
`docs/verification.md` laws 1-39); bookspec at `74f2853d` (also the tip of
its `main` today, checked with `git ls-remote`). The goal, from D-34 (the
owner, 2026-10-20: "bookspec is a SPEC first, and then it can also include
formal methods/validation/proofs as well as a test harness"): bookspec is
the definitive accounting specification, usable by anyone building an
engine; wanderbooks' code is sourced from it and reflects it accurately.

How to read the evidence: "verified" means I ran it (the `require` build,
the Lean decidability and soundness probe, the TOML probe, the file diffs,
the example census). Everything else is read from the files named.

---

## 0. The picture in ten lines

1. The real content is still in wanderbooks. bookspec's `spec/` is 668
   lines, one MUST-paragraph per rule (25-100 words) against rules.md
   entries of 9-398 lines. 17 of 45 rules are *fully* stated in bookspec
   (B-1 to B-11, B-17, B-20, B-21, B-22, B-28, B-31); 25 are summaries;
   3 are placeholders (B-39, B-40, B-43), although rules.md has stated
   them since P4b/P4.
2. bookspec's own docs say the opposite of the goal: its README ends with
   "the vendored authorities ... and `docs/spec/rules.md` stay in
   wanderbooks". Its pin check (`every_citation_resolves`) cannot read the
   sources, so it only checks the form of a pin; the verbatim check runs in
   wanderbooks (`every_vendored_pin_is_verbatim`). bookspec on its own
   cannot prove a single citation.
3. Lean: bookspec's `Basic`, `Entry`, `Statements`, `Closing` are
   byte-identical to wanderbooks' once `Bookspec`/`Wanderbooks` is
   normalized (verified with `diff` after `sed`). `Ledger.lean` differs by
   exactly one constructor, `discard`, and its three theorems (43 lines).
   Both packages use `leanprover/lean4:v4.34.1`, Lake manifest 1.3.0,
   `lakefile.toml`, no dependencies. Wanderbooks requiring bookspec works
   today (verified: a scratch project with a git `[[require]]` with
   `subDir = "lean"` built `Bookspec.Laws` in 15 s).
4. Of the 118 audited theorems, about 112 are engine-independent
   accounting laws (they belong in bookspec); about 6 are about
   wanderbooks-style no-figure annotations (`record`, `discard`, `markSent`)
   and even those state a general law.
5. The Lean `Step` relations are decidable per hypothesis and a `step`
   function is routine (verified on `addAccount` and `post`: decidability
   instances for `Accepted`, `closedOn` and all of `close`'s hypotheses
   elaborate with `infer_instance`; `step_sound` for those two constructors
   is 14 lines, standard axioms only). But the model is looser than the
   engine in four places (reverse-once, reversal date, empty close, ids),
   so an executable would first expose, then need, model fixes.
6. A Lean executable can read bookspec's TOML natively: Lake's own
   `Lake.Toml` parser works from a plain `lean_exe` (verified: it parsed
   `cash-flow.two-member-llc-first-year.toml`, 23 events).
7. Only 8 of the 66 examples are inside what any reference model has
   (entries, reversals, a retained-earnings close, bills and invoices;
   queries `trial_balance`, `balances`, `profit_and_loss`, `entry`). Those
   are the same 8 the Rust `derive.rs` model and the GnuCash adapter cover.
8. bookspec's schema and `Engine` trait are wanderbooks-shaped
   (`setup.parties`, `tin_last4`, `instruments`, `payees`, `projects`, IIF
   queries; `bill`/`invoice` events post to hard-coded accounts 6999, 2000,
   1200, 4000). An engine builder from scratch must adopt wanderbooks'
   product vocabulary to run the examples at all.
9. 14 rules have no example (B-17, 18, 20, 22, 24, 27, 28, 29, 39, 40, 42,
   43, 44, 45); 11 of those are stated rules the schema cannot yet show.
10. The rule list lives in four places that a test keeps in step by hand:
    `rules.rs` (the catalogue), `rules-index.md`, `spec/<area>.md`, and
    rules.md's `**bookspec:**` lines. Three of them are stale on B-39,
    B-40, B-43.

---

## 1. Rules

### 1.0 Classes and the completeness scale

Class (primary, then secondary where the rule mixes):

- **(a)** accounting law/rule any engine must follow;
- **(b)** accounting CONVENTION/CHOICE: bookspec names the options, an
  engine declares its pick (`Claims::choices`);
- **(c)** file/interchange format;
- **(d)** product/engine rule specific to wanderbooks.

bookspec today: **full** = the statement states all of the rule's
engine-independent content; **summary** = one paragraph of a longer rule
(says what is missing); **reserved** = placeholder (`Status: reserved`).
"Stays" = the wanderbooks claims (Enforced/Checked lines, code names,
journal events, MCP tools, viewer) that remain in wanderbooks after the
move. The tally of primary classes is under the table.

Every rule's rules.md entry has the same skeleton: `**bookspec:**` id,
`**Status:**`, authority/discussion, `**Enforced:**` (code names),
`**Checked:**` (test, Kani and Lean names). Rules B-38 to B-45 also carry a
```` ```text GOAL/SOURCE/INVARIANTS/SCENARIOS ```` block, the nearest thing to
bookspec's "symbolic form" and the only rules.md text that already has the
shape bookspec wants for invariants and scenarios.

### 1.1 The 45 rules

| B | BS id | Class | bookspec today | What is missing in bookspec | Stays in wanderbooks / split note |
|---|---|---|---|---|---|
| 1 | entry-1 | a | full | the CHOICE (2026-10-13) that a line's account must exist and take postings *when the line is added*, not only at post; no `too_few_lines` example (refusal word exists) | Enforced `kernel::check_lines`, `JournalEntry::validate`, `Ledger::add_line`; Kani `check_lines_is_exact`, `accepted_entry_nets_to_zero`; Lean `Accepted.net_zero` (a bookspec law); property `prop_posted_entries_balance` |
| 2 | entry-2 | b | full | none | Enforced `kernel::check_lines` (`NegativeAmountLine`); Kani; test `negative_amount_line_is_rejected` |
| 3 | ledger-1 | a (single money, AUTHORITY) + b (minor-unit booking CHOICE) | full | foreign-line example and schema (`rate`, original amount kept); rate source/table; "a new asset's cost is in the minor unit too" (an engine corollary) | `Ledger::add_line_fx`, `post_entry` (payroll/close/disposal lines), `replay_post_entry`, `validate_new_asset`; the GnuCash cross-check history; `fx_overflow_is_an_error` |
| 4 | ledger-2 | a (elements) + b (five-type chart, child takes parent's type) | full | none (CON 8 paragraphs are *cited* not vendored: fine) | `AccountType`, `Ledger::add_account` (`ParentTypeMismatch`) |
| 5 | ledger-3 | b | full | none | `AccountType::normal_side`, `kernel::normal_balance`; Kani `normal_balance_is_signed`; Lean `balance_after_line` |
| 6 | statement-1 | a | full | none | `reports::trial_balance`; Kani `trial_balance_columns`; Lean `trial_balance_iff`, `Reachable.trial_balance[_through]` (bookspec laws) |
| 7 | statement-2 | a | full | "unclosed income is part of equity on a live book" is only in the symbolic form (`A = L + E + (R - X)`); fine | Kani `accounting_equation`; Lean `accounting_equation`, `Reachable.accounting_equation[_through]`; `prop_balance_sheet_balances` |
| 8 | ledger-4 | a | full | the statement reaches the audit trail (Rev. Proc. 98-25) but not the *machine-sensible* property in a testable form | hash-chained append-only journal, `journal_tamper_is_detected` (class d: stays); Lean `Step.extends`, `Steps.extends` are bookspec laws |
| 9 | ledger-5 | a | full | the "H"-mark / linked-by-`reversal_of` is engine vocabulary: only "linked to the original" is stated, correctly | `Ledger::reverse_entry`; Kani `reversal_mirrors`, `reversal_cancels`; Lean `accepted_reversal_iff`, `movement_with_reversal` |
| 10 | ledger-6 | b (CHOICE: reversible once / never closes) | full | the options are named in the Status line but only one pick is demonstrated; the Lean model does not enforce it (see 5.3) | `Ledger::check_reversal`; `an_entry_is_reversed_at_most_once`, `a_closing_entry_is_never_reversed` |
| 11 | period-1 | a (mechanics) + b (which account the net lands in) | full | the two-step through a profit-and-loss account (Pacioli ch. 27/34) is only implied | `Ledger::close_period_to`; Kani `closing_line_zeroes`, `closing_entry_balances`; Lean `closingEntry_*`, `close_zeroes`, `close_credits_net_income` (law 4) |
| 12 | period-2 | b | summary | the CHOICE (2026-10-13) that periods nest and never partially overlap has no source line and no example; "a period with an open draft cannot close" (Lean `hdrafts`, golden `close_refused_with_draft`) is not stated | `closed_period_containing`, `CloseOutOfOrder`, `Store::rehearse`, store property `refused_commands_never_reach_the_journal` (stays); Lean `Steps.frozen` (law) |
| 13 | partner-1 | a (shares, capital accounts, §706(d) counts) + b (cent rounding; counted from first close, D-10; statement splits unclosed earnings the same way, D-38) | summary (75 words of 153 lines) | the cumulative-count algorithm (count definition, restart on member/share change, true-up, "each close hands out exactly its net", the < 1 unit per member per count bound and its precondition), the capital-account roll-forward statement, corrections netting out (`cancelled_in_span`), contributed property vs cash (CHOICE 2026-10-15), statements chaining to the unit; sources §704(a), Reg. 1.704-1(b)(2)(iv)(b), §706(d)(1), Reg. 1.706-4(a)(3)(vii), 1065 Instructions Item L; invariants; scenarios | split: **stays** the journal feature `members_cumulative`, replay of `cumulative` / `largest_remainder` / last-absorbs legacy closes, `Store::close_period_to`, MCP `close_period` output, viewer pages, 28 named checks. **Moves**: everything before "Enforced" except the "Journaled `cumulative_since`" and "Closes journaled ..." bullets. Lean `Apportion.*`, `Closing.*` are the laws |
| 14 | statement-3 | a (ASC 230 mechanics) + b (CONVENTION order of cash allocation; CHOICE D-42 constructive cash) | summary (73 words of 398 lines, the largest rule) | all 12 numbered rules: partition of non-cash accounts, part-cash allocation order, gains/losses on investing assets, gross lines by kind and class (a)-(g), governments split by share, payroll stays (d), reversals, noncash disclosure formula `x = max(0, min(D-out, C-in))`, NI reconciliation `operating = NI + G + X + W`, footing, on-behalf cash (D-42) with its source discussion, equipment paid later; ASC paragraphs 230-10-45-1..31, 50-3..8; invariants; scenarios | **stays**: `books.cash_accounts` / `financing_accounts` / `investing_accounts` settings and `role_problems` refusals (these are engine configuration; bookspec's `setup.cash_accounts` etc. is the portable form), flags `asset_on_operating_payable` / `asset_sold_on_operating_receivable` as the statement's `warnings` and Needs attention (product), printed-statement/viewer row layout, `books.cash_flow` and exports, the 55-name Checked list. **Split**: rules 1-12 are accounting; "Which accounts those are is the books' `books.cash_accounts`" and rule 5's viewer/printed sentence and the `books.add_vendor`/`books.assign_role` remedies are product |
| 15 | asset-1 | b (straight line by days: CHOICE 2026-10-13/20) + a (ASC 360-10-35-4 allows any systematic method) | summary (102 words of 188 lines) | rules 1-8: life interval, mid-month formula, last month takes the remainder by days, half-away rounding of each *cumulative* figure, disposal semantics (excess/final depreciation, refusal while later runs stand), run catch-up and reinstatement limits, two dates (in service vs acquired), posted vs scheduled; Pub. 946 tax conventions are explicitly out of scope; 13 examples exist, none labelled with their rule-number | **stays**: `Store::run_depreciation`, `dispose_preview`, `record_depreciation_run`, `live_depreciation_run`, `Ledger::depreciation_runs`, `books.list_assets`, viewer asset page, B-33 cross-reference. **Split**: rule 6's "commands only, journals replay as written" and rule 7's "a new link ... is refused" are command-surface (d); the figures are (a)/(b). Kani `prorate_*`, `catch_up_*`, Lean `Prorate.*` are the laws |
| 16 | prepaid-1 | a (revenue/contract liability) + b (prepaid expensing; `daily` vs `monthly_even`) | summary (70 words of 61 lines) | monthly_even start-on-the-1st rule (CHOICE D-37), thirteen-month-ends example for a mid-March start, closed-period month "left for a human", no reversal dated before its month, the unvendored ASC 606-10-25-31/32, 55-20 | `schedules::*`, `Ledger::schedule_month_hold`, `books.schedule_convention` (a setting: product), legacy schedules keep equal months "forever" (replay), `mcp_daily_schedule` |
| 17 | revenue-1 | a | full | example: needs a `payout` event (gross, fee, deposit account) or a settlement event (BS-arap-6 shows the same gross presentation) | `payouts::payout_plan` (stays); `books.record_payout` |
| 18 | arap-1 | b | summary | as-of semantics (documents issued by then, payments made by then, a payment never predates its document, reversals), "the buckets partition what the documents owe, exactly", due-today-is-Current CHOICE; example needs `ar_aging`/`ap_aging` queries | `arap::aging_bucket`, `Ledger::ar_aging/ap_aging`, `PaymentBeforeDocument`; FFIEC note |
| 19 | 1099-1 | a (§6041, §6041A, Pub. L. 119-21 §70433, §1.6041-1/-3, §1.6050W-1) + b (D-40 dated decisions, shares CHOICE, "presumed check" CHOICE) | summary (99 words of 217 lines) | who pays; card/network exclusion reasoning; the 16-case payment-bucket table and netting; undetermined vs not-reportable; dated decisions; the status function (lo/hi of unknowns); shares across parties; threshold table by year and the "refuse past the known years" rule; invariants and scenarios | **stays** (product): `instrument.added`/`entry.instrument_set` events, `books.set_vendor_1099`, `books.set_entry_instrument`, "the quieter note" (untagged-entry digest), `RuleNotKnown` review dates and the Needs-attention flag (a registry policy: the *years known* belong to the spec as data, the staleness alarm does not), `settings::Source`. **Moves**: threshold, payor, bucket table, decision/status semantics |
| 20 | ledger-7 | a (records must be kept) + d (enforcement is the MCP receipt policy "not the engine") | full | no example, and no executable form: a *procedural* rule | `ReceiptPolicy`, MCP receipt gate (R-13). Recommend a rule class "procedural" that needs no example |
| 21 | entry-3 | a | full | none | `Decimal` money, `Amount` checked arithmetic; Kani `sums_are_exact`; bridge tests |
| 22 | asset-2 | a/b (de minimis safe harbor: $500 / Notice 2015-82 $2,500; 12-month rule) | full | no example; policy not engine behaviour ("bookkeeper policy, AGENTS.md") | none (it is the books' own AGENTS.md); procedural |
| 23 | ledger-8 | b (control roles) + d (default codes 1200, 4000, 2000, 6999, 3999, 6100, 6110, 2110-2150 and the role list are wanderbooks') | summary | the movability rule (a receivables, payables, withholding or retained-earnings role moves only when its account is empty), "no account fills two roles", "the default-code account is the role's only once a document posts", the roles' account kinds | `Ledger::check_controls`, `assign_role`, `check_renumber_account`, `role.assigned` journal events (stays); **split**: the default codes and the role list are engine configuration, expressed in examples as setup not as rule text |
| 24 | asset-3 | b/d (tags: 1-32 chars `PREFIX-0001`, serial: product) + a (register ties to GL: three tie-outs) | summary | the register reconciliation (`unregistered_cost`, `unposted_depreciation`, `unattributed_accumulated`), tag uniqueness ignoring case and never reuse; no tag example | `set_asset_tag`, `check_asset_tag`, `check_serial`, `books.label_asset`, labels (class c: Avery 5160); the reconciliation is exercised by existing examples (`register_totals`) but not attributed to this rule in the catalogue |
| 25 | asset-4 | a (§1012, §723, §704(c), Reg. 1.168(i)-4) + b (D-41: lesser-of for personal-use conversion, traditional method) | summary (56 words of 78 lines) | the purchase / carryover_723 / personal_use_conversion rules, the built-in-gain definition and fixing at contribution, §704(c)(1)(C) built-in loss only for the contributor, "facts are kept, figures derived" | `AssetBasis::check`, `asset.basis_set` events, `replay_set_asset_basis`, `books.register_asset`, `books.set_asset_tax_basis`, Kani `basis_figures_laws` |
| 26 | partner-2 | a (truncated TIN, Reg. 301.6109-4) + b (party directory, roles, unique names) + **d (≈80% of the entry)** | summary | the accounting part is small: one directory of parties with roles; vendor/customer records link to a party; W-9 facts need a 1099 role, W-4 facts the employee role; a party field holds at most the last four digits of a TIN | **stays**: the encrypted vault, `books.set_w9/set_tin/get_w9/purge_tin`, the TIN-file registry and text scan, its 100,000-candidates arithmetic, redaction in refusals, Unicode separators (`sources/unicode-tin-text.md`), downloads (D-44/D-46), the access log, links rule. **Split recommended**: bookspec BS-partner-2 = "no full TIN in the books; last four; directory; roles"; the rest becomes wanderbooks product rules (WB-xx) |
| 27 | format-1 | d (journal format gate) | summary | bookspec calls it a CHOICE "interchange format" but bookspec defines no journal format. The principle is generic: an older reader never silently misreads or skips newer records and never calls them corrupt | **all of it stays**: `journal.format_set`, `KNOWN_FEATURES`, SQLite projection files, upgrade with the owner passphrase, 229 lines. Recommend retiring `BS-format-1` into a one-sentence generic rule (BS-ledger-4 corollary) and moving the rest to wanderbooks product rules |
| 28 | entry-4 | b (four-digit years, 10^15 amount, ten decimals) + d (replay accepts what earlier versions did; MCP panic-safety; 25 `replay_*` names) | full for the core | no example (a refusal needs a date or amount the schema cannot spell) | everything from "Replay accepts every line ..." on, `ledger_core::calendar`, `hostile_args.rs`; the bounds themselves (`MAX_LINE_AMOUNT`, `MAX_LINE_SCALE = 10`) are CHOICEs an engine may pick differently |
| 29 | ledger-9 | d (settings, conventions, decisions, open items, approvals, in the journal) + a (a record of how the books are kept moves no figure; the fiscal year cannot move once a period is closed under it, Form 1128) | summary | nothing accounting-relevant beyond those two sentences; Lean `Step.record_changes_no_figure`, `Reachable.records` | the rest is product: `settings.set`, `convention.set`, approval rules, `books.set_setting`, settings files import/export (`ledger_config::codec`), `fuzz_settings_replay` |
| 30 | period-3 | a (periods) + b (the omitted-date default) | summary | the table of report kinds (period / instant / calendar year), partial-date rules, first-year note, "today" is the company's, CPA package and "Year to date" preset are the fiscal year of their day | `ledger_core::calendar::PeriodDefaults`, `books.show`, tool descriptions, viewer `default_period`; `company.timezone` / `company.fiscal_year_start` are settings (product) |
| 31 | ledger-10 | b | full for the rule | the sibling-name and nine-digit refusals cite an *interchange format* (IIF: "accounts are identified by NAME") for a ledger rule | `account.renamed` journal event, `books.rename_account`; the IIF citation is class (c) leaking into (b) |
| 32 | ledger-11 | a (records must show the return's lines) + b (warn vs refuse; header exemption; D-67 deductible amount rounds once) | summary | warn/refuse policy semantics, role accounts never refused, headers exempt, uncategorized listing, **D-67** (`Deductible` law), the category catalogue itself (`tax::CATEGORIES`, 460 lines of `irs-tax-lines.md`) is accounting reference data bookspec does not have | `books.add_account`, `books.import_chart_csv`, `books.set_tax_category`, starter charts (`templates.rs`), `policy.require_tax_category` setting |
| 33 | statement-4 | b (a report as of a day uses the books through that day) + d ("CPA package") | summary | "the 1099 worksheet counts the calendar year from January 1 through as_of" (CHOICE D-36), which accounts one list serves | `books.export_cpa_package`, the viewer's export; **split**: "package" is product, the as-of principle is general |
| 34 | format-2 | c (IIF) | summary (84 words of 161 lines) | the numbered rules: one transaction per entry through as_of, closes left out, ASCII with transliteration list, 31-char names, subaccount paths, customer/vendor on A/R and A/P, classes, CR LF, DOCNUM; Intuit pins | `ledger_import::iif`, `ledger_package` export; stays an appendix with its own adapter (`the_bookspec_iif_example_matches_the_engine`) |
| 35 | partner-3 | b (CONVENTION sign; CHOICEs for what counts as money) + a (Reg. 1.704-1(b)(2)(iv), Item L, Helvering v. Price) | summary (56 words of 230 lines, 9 items) | sign, direction/cash/in-kind, what the shares split (target order), closes allocate nothing, reversals, cards and the members' financing, loans, party totals bases (`cash` / `all`), 1099 vs party totals (item 9) | `shares::*`, `parties::*`, `books.parties`, `books.party`, `books.set_entry_party`, viewer party pages. Lean `Parties.*` are laws for this rule (they mirror `kernel::party_target`) |
| 36 | partner-4 | a | summary | the Item L transactional approach line by line, §723 carryover, D-38 running split for property shares, book-vs-tax reconciliation causes (`built_in_gain_704c`, `property_without_basis`, `property_distributed`) | `ledger_core::tax_capital`, `k1` worksheet code, CPA package K-1 sheet |
| 37 | statement-5 | b (a project is a class: CHOICE; job costing CONVENTION) + a (ASC 450-30-25-1, 340-40-25-5/-7) + **d (projects page facts, links, notes, text rules)** | summary | item 4 (on-spec costs are expenses), item 8 (document lines carry a project), the figures `net`/`to_date`/`unrecovered_cost`; | **stays**: project facts (description, status, kind, party, dates, ≤8 links, notes), `books.add_project/update_project/projects`, viewer. **Split**: bookspec keeps "a class tag on lines; figures are the tagged lines"; the facts are product |
| 38 | arap-2 | b (documents are first-class; control account = its documents) + a (subledger tie-out) | summary | the five numbered items: tags beside lines, figures as signed sums of tagged lines, reversals mirror tags, dating rules, the control guard (hand lines refused), legacy lines; GOAL/INVARIANTS/SCENARIOS block exists in rules.md and is the model for bookspec's | `Ledger::doc_state`, `control_check`, `check_control_lines`, journal replay deriving tags (stays); Lean `Documents.*` (law 34) |
| 39 | arap-3 | b (CONVENTION: trade-terms notation) | **reserved** | whole rule (rules.md has GOAL/SOURCE/INVARIANTS/SCENARIOS); pure function: Net n, Net n EOM, due on receipt, custom, discount_by; `quantity x unit price = amount` exactly; line account kinds | `calendar::check`, `kernel::line_amount`, `kernel::discount_max`; no example exists anywhere (the plan notes `payment-terms-due-date` was "recorded as next step") |
| 40 | arap-4 | b (sequential invoice numbers, voids kept; duplicate-payment control) + d (issued PDF filed with sha256, `receipt.attached`) | **reserved** | whole rule; invariants 1, 2, 4, 5 are accounting convention, 3 (the PDF copy and its hash) is product | `invoice.sent`, `receipt.attached`, `vendor_ref` normalization is a heuristic (b) |
| 41 | arap-5 | a (§166, §111, Reg. 1.166-1/-2/-3) + b (CHOICE: direct write-off; ASC 326 allowance noted) | summary | the numbered items (write-off entry shape, the day, cash basis refused, partial write-off, recovery is income, evidence of worthlessness); has an example | `write_off_invoice`, `invoice.written_off` event, the evidence filed in the same call (d) |
| 42 | arap-6 | b (settlement exactness; gross discount; gross fee) | summary | one party one kind, bounds on every day, discount cap `discount_max` and reason, fee gross, the refused discount on a bill with asset lines (D-49); **no example** | `kernel::settle`, `payment_room`, `receive_payment`, `pay_bill`; Lean `settle_posts_a_document_step`, `settle_unattached` |
| 43 | entry-5 | b/d (drafts; two states `building`/`awaiting_evidence`; discard) + a (Pub. 583: support for entries) | **reserved** | whole rule (rules.md has the spec block): one function decides whether evidence is required, exempt-by-origin list, D-47 reversals need none, states move no figure, discard only a draft, a period with a draft cannot close; Lean `Step.discard_*` | `evidence::requirement`, `Store::post_entry_checked`, `ReceiptRequired`, `entry.draft_state_set`, `entry.draft_discarded`, MCP/viewer; the golden `draft_states_and_evidence_gate.json`. **Split**: "drafts hold a period open; a discard moves no figure; a posted entry is never discarded" is general (the Lean model already has `drafts`); the receipt policy and states are product |
| 44 | ledger-12 | d (evidence extraction records, OCR refusal D-15, duplicate withdrawal, "a payment says what it settles") | summary | the generic principle ("derived records are never authoritative over the bytes; identical bytes are one file") is two sentences; the rest is wanderbooks' pipeline | `attachment.text_extracted`, `entry.settles`, `attachment.withdrawn` (all stay); a profile "evidence" if bookspec wants it, else retire |
| 45 | ledger-13 | d (audit as a deterministic run, attestations, the owner's bar D-9, CPA package gate) | summary | generic principle: an audit is a pure function of the journal's figures, a check that cannot be made is a finding, never a silent pass. The checks, severities, workpaper bytes, attestation bar are code's | `ledger_core::audit`, `books.audit`, `audit.run`; golden `audit_workpaper.json` pins the code's check list: product |

Tally of primary class (B-numbers): **(a)** 1, 3, 4, 6, 7, 8, 9, 11, 13, 14, 17,
19, 20, 21, 25, 36, 41 = 17; **(b)** 2, 5, 10, 12, 15, 16, 18, 22, 23, 30, 31,
32, 33, 35, 37, 38, 39, 42 = 18; **(c)** 34 = 1; **(d)** 24 (asset tags), 26,
27, 28, 29, 40, 43, 44, 45 = 9. Rules that mix and need a split: 13, 14, 15,
19, 23, 24, 26, 28, 29, 32, 33, 37, 40, 43. Rules that should leave bookspec's
numbered rules (or become one generic sentence each) on product grounds: B-27,
B-29 (but for two sentences), B-44, B-45, and the vault half of B-26.

### 1.2 How to split a mixed rule (the pattern)

Use one rule shape in both repositories and make it mechanical:

- bookspec: `### BS-<area>-<n>` with **Statement** (numbered MUSTs), **Sources**
  (pinned), **Invariants**, **Scenarios**, **Examples**, **Applicability**, and a
  new **Options** field for CHOICE rules (the rules.md prose "CHOICE (the
  owner, 2026-10-13)" becomes a named option plus who/when as provenance, not
  as part of the statement).
- wanderbooks `docs/spec/claims/<area>.md` (see unit D): for each BS id,
  `Claims: yes/partly/no`, `Pick:` (which option), `Enforced:`, `Checked:`,
  `Product rules:` (the WB-n rules that the BS rule does not cover).

The test for a sentence: does it still make sense if the engine has no MCP
server, no viewer, no SQLite and no journal? If yes it is bookspec. Words that
mark product text: `books.*`, "the viewer", "journal" (as an event type),
`ledger_*` crate names, `Needs attention`, "the owner" as the person who
chooses a *default* (provenance of a CHOICE may stay as a date line).

### 1.3 Notable inconsistencies found in the catalogue

- rules-index.md and `rules.rs` call B-39, B-40, B-43 "allocated, not yet
  stated" but rules.md states all three (P4b S4, P4); plan/served.md 8.3.x
  notes this ("`bookspec/spec/rules-index.md` still lists them ... not
  touched"). `rules_md_carries_every_id` skips rules without a status, so the
  gap is invisible to CI.
- B-24's catalogue entry lists no example although `register_row`,
  `register_totals` examples exercise the register reconciliation, and the
  README's query table cites B-24 for them.
- `BS-format-1` (journal gate) and `BS-ledger-12/-13` (evidence, audit) are
  product rules numbered as if they were engine-independent.
- bookspec's page budget is 60 pages x 55 lines = 3,300 lines
  (`the_edition_fits_its_page_budget`). rules.md alone is 3,633 lines of rule
  text; a complete bookspec needs the budget raised (or per-area files outside
  the budget).

---

## 2. Sources

27 files in `docs/spec/sources/` (plus `README.md` and the licence note
`us-public-domain-status.md`); the brief said 24. "Rules (rules.md)" are the
rules whose entry names the file; "bookspec pins" are the `vendored` lines in
bookspec's `spec/*.md`. Code citations are the number of Rust/doc files naming
the file.

| File | Lines | Rules (rules.md) | bookspec pins | Class | Licence as the file/README states it | Can move to bookspec as is? |
|---|---|---|---|---|---|---|
| pacioli-1494 | 230 | B-1 (by file); B-2, 3, 5, 6, 8, 9, 11 (by "Pacioli ch.") | B-1, 2, 3, 5, 6, 8, 9, 10, 11 | accounting authority | public domain (published 1494 / 1914), stated in the file | yes |
| irs-records | 34 | B-1, 8, 20, 31, 32, 43 (also B-44, 45 "Rev. Proc. 98-25") | B-1, 6, 8, 20, 31, 32, 44, 45 | accounting authority | public domain (17 U.S.C. §105; README table), verbatim | yes |
| irs-periods-and-methods | 27 | B-12, 30, 33 | B-12, 30, 33 | authority | PD, verbatim | yes |
| irs-1099-thresholds | 52 | B-19, 30, 33 | B-19 | authority | PD, verbatim (U.S. Code via Cornell LII) | yes |
| irs-1099-payment-method | 399 | B-19 | B-19 | authority | PD, verbatim (statute, regs, instructions) | yes |
| irs-capitalization | 25 | B-16, 22 | B-16, 22 | authority | PD, verbatim | yes |
| irs-depreciation | 19 | B-15, 25 | none | authority (tax conventions the book method is distinct from) | PD | yes |
| irs-partnerships | 125 | B-13, 21, 36 | B-13, 21, 36 | authority | PD, verbatim | yes |
| irs-capital-accounts | 38 | B-35 | B-13, 35 | authority | PD, verbatim | yes |
| irs-partnership-basis | 104 | B-15, 25 | B-25, 36 | authority | PD, verbatim | yes |
| irs-truncated-tin | 11 | B-26 | B-26 | authority | PD, verbatim | yes |
| irs-tin-formats | 16 | none (code: `ledger_vault`) | B-26 | authority/product (what makes a TIN) | PD; no licence sentence in the file | yes after adding a licence line; it governs the vault, so it may stay as a product source |
| irs-tax-lines | 460 | B-32 | none | authority (return line items: Schedule C, 1065, 1120-S, 1120) | PD, verbatim | yes, and it should: it is the reference for the tax-category catalogue (`tax::CATEGORIES`) which is data bookspec lacks |
| irs-entity-classification | 52 | none (code: settings, ledger-config) | none | authority (Reg. 301.7701-3) | PD | yes (feeds the "tax classification" setting; not cited by a rule yet) |
| irs-payroll | 15 | none (code: payroll in `ledger.rs`) | none | authority (Pub. 15 rates) | PD | yes, but cited by no rule: payroll has no BS rule at all |
| helvering-v-price-1940 | 11 | B-35 | B-35 | authority (case law: a promise is not payment) | no licence sentence in the file; a U.S. Supreme Court opinion is not copyrightable (the README table does not list case law) | yes after a one-line licence statement |
| us-public-domain-status | 9 | none (explains the IRS files) | none | licence note | cites 17 U.S.C. §105(a) | yes (it is the licence statement for all the PD files) |
| fasb-asc | 322 | B-14, 15, 16, 17, 37 (and B-18, 38, 41, 42 cite ASC without the file) | B-14, 15 | authority, third-party copyright | "copyrighted by the Financial Accounting Foundation ... keeps only short quotes and paraphrase" (stated in the file) | yes, same terms; bookspec's CC BY 4.0 text may *cite* it, not copy |
| iso-4217 | 24 | B-3, 15 | B-3 | authority (currency minor units) | ISO's list is not reproduced; facts only (stated in the file) | yes |
| unicode-tin-text | 65 | B-26 | none | product (how a TIN can be written, for `has_full_tin`) | "Unicode License v3, which permits copying" (stated) | stays: serves only the vault/TIN scan |
| intuit-iif | 238 | B-31, 34 | B-31, 34 | format | © Intuit; cited with short quotes + URLs + hashes (stated) | yes, as the formats appendix source |
| intuit-bank-csv | 88 | none (code: `bank_csv.rs`) | none | format (import) | © Intuit; cited, short quotes (stated) | stays unless bookspec grows an import appendix |
| xlsx-numbers | 76 | none (code: `xlsx.rs`) | none | format | © Ecma / © Microsoft; cited, short quotes (stated) | stays (product export) |
| avery-5160 | 49 | none (code: `labels.rs`) | none | format/product (label geometry) | © Avery; cited (stated) | stays |
| file-names | 34 | none (code: `ledger-gui` export) | none | product | Microsoft docs and POSIX; no licence sentence in the file | stays |
| jsonrpc-2.0 | 37 | none (code: `ledger-mcp`, `ledger-host`) | none | product (wire protocol) | no licence sentence in the file | stays |
| mcp-tools | 48 | none (code: `ledger-mcp`) | none | product | "(MIT)" stated for the MCP repo | stays |

Findings:

- **Move**: the 16 accounting-authority files (Pacioli, the 13 `irs-*`
  except `irs-tin-formats`, Helvering, `fasb-asc`) plus `iso-4217`,
  `us-public-domain-status` and `intuit-iif` (formats appendix): 19 files,
  about 2,200 lines. The brief's 24 lands at 27 on disk.
- **Stay** (product/format): `unicode-tin-text`, `intuit-bank-csv`,
  `xlsx-numbers`, `avery-5160`, `file-names`, `jsonrpc-2.0`, `mcp-tools`,
  `irs-tin-formats` (8 files).
- **Licence hygiene before moving**: bookspec's `spec/` and `examples/` are CC
  BY 4.0; `sources/` would need its own line in the README licence table:
  "public-domain U.S. government texts, verbatim; third-party works cited with
  short quotes under their owners' terms". `helvering-v-price-1940`,
  `irs-tin-formats`, `irs-entity-classification`, `irs-payroll`, `file-names`,
  `jsonrpc-2.0` have no licence sentence of their own and rely on the README
  table or on silence. Add one line each when moving.
- OpenStax (CC BY-NC-SA, "incompatible with this repo's license") is cited by
  B-5, B-11, B-15, B-16 with short quotes and attribution; moving that to a CC
  BY 4.0 repository keeps the same short-quote terms, but state it.
- Raw retrieved files are not stored, only their SHA-256 and the extracted
  verbatim text; bookspec inherits that practice.
- Nothing cites the three new accounting-reference gaps (payroll, entity
  classification, tax lines) from a bookspec rule: moving them without rules
  would add sources with no citing rule. Either write the rules (payroll
  withholding mechanics, B-32's category catalogue) or leave them until
  those rules exist.
- 346 `SOURCE:` lines and 336 `GOAL:` blocks in the Rust code cite
  `docs/spec/sources/...` or `rules.md B-n`; moving the sources changes about
  60 of them (the ones naming a source path) and the 784 `rules.md B-n`
  mentions in 160 files (see unit E).

---

## 3. Lean

### 3.1 File by file

| wanderbooks file | Lines | What it is | bookspec has it? | Class |
|---|---|---|---|---|
| `Basic.lean` | 165 | Sides, line, net | identical modulo namespace (verified) | law, generic |
| `Entry.lean` | 117 | `Accepted`, movements, reversals | identical (verified) | law |
| `Statements.lean` | 683 | balances, trial balance, accounting equation, cash-flow partition, `constructive` (D-42), NI reconciliation | identical (verified) | law |
| `Closing.lean` | 451 | closing entry, `plugLast`, cumulative members split, true-up, statement unmoved by a close | identical (verified) | law (B-11, B-13) |
| `Ledger.lean` | 605 | the `Step` relation, `Reachable`, preservation, frozen periods, records, discard | **differs** (562 lines): bookspec lacks `Step.discard`, its preservation cases (`inv`, a `⟨[], ...⟩` case) and `Step.discard_keeps_books`, `Step.discard_drafts`, `Step.discard_unblocks_close` | law; `discard` is a generic draft law |
| `Apportion.lean` | 509 | largest remainder for any number of parts, drift, weighted, cumulative | **missing** | law (B-13 CHOICE) |
| `Parties.lean` | 215 | party target: odd, reverse involutive, reversal moves the opposite | **missing** | law of a CHOICE (B-35); mirrors `kernel::party_target`, so its definitions are engine-shaped |
| `Prorate.lean` | 324 | days proration, rounding near the exact figure, monotone, last month remainder | **missing** | law (B-15, B-16) |
| `Deductible.lean` | 330 | shares added exactly, rounded once, half away, at any size | **missing** | law (B-32 D-67, B-21) |
| `Documents.lean` | 395 | control accounts equal their documents; write-off; settle; reverse; markSent | **missing** | law (B-38, 41, 42); `Send`/`markSent` product |
| `Audit.lean` | 126 | prints axioms of 118 theorems | bookspec's has 5 | tool |
| `Wanderbooks.lean` | 10 | root imports | `Bookspec.lean` imports `Laws`, `Ledger` | tool |

bookspec's extra: `Laws.lean` (62 lines): names the five laws
(`law_debits_equal_credits`, `law_accounting_equation`, `law_trial_balance`,
`law_net_income_articulates`, `law_cash_flow_is_change_in_cash`) as thin
aliases over wanderbooks' `Reachable.books_net_zero`, `.accounting_equation`,
`.trial_balance`, `close_credits_net_income`, `.cashflow_is_change_in_cash`.
(The audit brief counted five ported files; there are five plus `Laws.lean`:
`Basic`, `Entry`, `Statements`, `Closing`, `Ledger`, and `Statements` is
also identical.)

Drift summary: wanderbooks has 5 extra files (Apportion, Parties, Prorate,
Deductible, Documents: 1,773 lines, 63 of the 118 headline theorems) and one
extra constructor; bookspec has `Laws.lean`. Nothing in bookspec's Lean is
absent from wanderbooks'.

### 3.2 The 118 headline theorems

Counted by file (the `Audit.lean` order, mapped by name):

| File | Theorems | Verdict |
|---|---|---|
| Entry | `Accepted.net_zero`, `sum_movements`, `movement_with_reversal`, `accepted_reversal_iff` (4) | bookspec (law 1, 5) |
| Statements | `balance_after_line`, `trial_balance_iff`, `net_splits_by_flow`, `applyCash_conserves`, `applyCash_exhausts`, `applyCash_neg`, `entry_flows_sum`, `flows_reversal`, `constructive_nets`, `constructive_neg`, `constructive_sections_net`, `stmt_flows_sum`, `stmt_flows_reversal`, `cashflow_sections_sum_to_cash_change`, `secNet_cash_is_movement`, `operating_reconciliation_ties`, `operating_reconciliation_ties_all` (17) | bookspec (laws 6, 7, 10; B-14) |
| Ledger | `accounting_equation`, `Reachable.accounting_equation`, `Reachable.inv`, `.books_net_zero`, `.trial_balance[_through]`, `.accounting_equation_through`, `Steps.extends`, `Steps.frozen`, `close_zeroes`, `close_credits_net_income`, `statements_add_up`, `cashflow_reversal_nets`, `Reachable.cashflow_foots`, `.cashflow_is_change_in_cash` (15) | bookspec (laws 8, 11-17) |
| Ledger (annotation steps) | `Step.record_changes_no_figure`, `Reachable.records`, `Step.discard_keeps_books`, `Step.discard_drafts`, `Step.discard_unblocks_close` (5) | generic law ("a note moves no figure"; "a discard moves only its draft"); belongs in bookspec too, modelled engine-neutrally (a `note` step, a `discard` step). **These are the only 'wanderbooks command' theorems; they are not wanderbooks-specific in content** |
| Closing | `closingEntry_net/_zeroes/_accepted`, `net_closings_is_net_income`, `plugLast_total/_keeps`, `runPart_telescopes`, `closePart_total`, `trueUp_total/_running`, `closePart_eq_trueUp`, `cumulative_drift`, `statement_after_close`, `statement_unmoved_by_close` (14) | bookspec (B-11, B-13) |
| Apportion | 9 (`ranks_perm` ... `allocW_cumulative_total`) | bookspec (B-13) |
| Parties | `reverseTarget_involutive`, `capitalTarget_odd`, `partyTarget_odd`, `sumOver_movement_reversal` (4) | bookspec if B-35 migrates (it should) |
| Prorate | 18 | bookspec (B-15, B-16) |
| Deductible | 20 | bookspec (B-32 D-67, B-21) |
| Documents | `control_agrees[_through]`, `Step.document_unattached`, `settle_posts_a_document_step`, `settle_unattached`, `Step.reverse_moves`, `reverse_twice`, `Reachable.unattached_zero`, `.control_is_documents`, `Step.writeOff_unattached`, `writeOff_within_bounds` (11) + `Step.markSent_changes_no_figure` (1) | bookspec (B-38, 41, 42); `markSent` is the same "annotation moves no figure" shape |
| **Total** | **118** | **~112 pure law, 6 annotation-step laws (generic shape), 0 truly wanderbooks-only** |

What stays in wanderbooks: not theorems but the *bridge* from the Rust to the
spec: Kani harnesses in `crates/ledger-core/src/proofs.rs`, the kernel bridge
properties in `tests/kernel_bridge.rs`, the Rust-name comments ("Mirrors
`kernel::flip`"), and `docs/verification.md`'s law-to-harness table. The Lean
docstrings all say "Mirrors `crates/ledger-core/src/kernel.rs` ..." and would
need to be rewritten engine-neutral when they move; the mapping to Rust names
becomes a table in wanderbooks.

Two independent state machines exist: `Ledger.Step` (on `Ledger`, with
`Entry`) and `Documents.Step` (on `Books`, with tagged lines `TLine`). There is
no refinement theorem between them. A unified bookspec model needs one state
type; see 5.4.

### 3.3 Could wanderbooks' `verify/lean` `require` bookspec's package?

Yes, with small work. Verified facts:

- `lean-toolchain`: both `leanprover/lean4:v4.34.1`.
- `lakefile.toml`: both `name = ...`, one `[[lean_lib]]`, no `[[require]]`;
  `lake-manifest.json` version 1.3.0, `packages: []`, `fixedToolchain: false`.
- bookspec's package lives in the `lean/` subdirectory of the repository.
  Lake supports that with `subDir`.
- Verified in a scratch project in the scratchpad (not in the repo):
  ```toml
  [[require]]
  name = "bookspec"
  git = "https://git.swaits.com/swaits/bookspec"
  rev = "74f2853d5f0db0066147c17fed8033cfe3d83a24"
  subDir = "lean"
  ```
  `lake build` cloned, checked out the rev and built `Bookspec.Basic` ...
  `Bookspec.Laws` and a consumer file importing `Bookspec.Laws` in 15 s
  total (`nice -n 10`). `#check @law_debits_equal_credits` resolved.
  (A `file://` URL to the local cargo git db failed: "unable to read tree";
  the real remote works.)

What it takes:

1. bookspec's Lean gains the five missing files and `discard` (unit F1),
   neutral docstrings, a namespace policy (everything in `Bookspec.*`;
   `Documents`, `Apportion`, `Prorate`, `Deductible` as `Bookspec.<X>`).
2. wanderbooks deletes its copies (`Basic`, `Entry`, `Statements`, `Closing`,
   `Ledger`, and the rest), keeps `Audit.lean` (which `import Bookspec` and
   prints the axioms of the headline theorems under their new names) and
   perhaps a small `Wanderbooks/Bridge.lean`.
3. Two pins must agree: `rev` in `crates/ledger-core/Cargo.toml` (twice) and
   the `rev` in `verify/lean/lakefile.toml` / `lake-manifest.json`. Add a test
   (in `bookspec.rs` or the traceability gate) that reads both and fails on
   a difference.
4. `just prove` needs network once (`lake build` fetches the git package) and
   a cache of `.lake/packages`; CI needs the same. `mise` already pins Lean.
5. `docs/verification.md` names theorems unqualified (`Accepted.net_zero`);
   they become `Bookspec.Accepted.net_zero`. 39 law entries to touch; the
   law numbering (1-39) should move to bookspec as the "laws" index
   (`spec/laws.md`) with wanderbooks mapping its Kani harnesses to law
   numbers.
6. `Ledger.lean` references `Wanderbooks.accounting_equation` explicitly (two
   places); `Documents.lean`/`Deductible.lean`/`Apportion.lean` use
   `namespace Wanderbooks.X` and `open Wanderbooks.Prorate`: rename.
7. Keep `Audit.lean` in bookspec as the canonical audit (all 118) and have
   wanderbooks run the same audit through its `just prove` (so the "standard
   axioms only" rule is checked once per repo that builds the package).

Risks: the git `require` couples every wanderbooks build to Gitea
availability (mitigate with the lake packages directory in the CI cache and
mise task); a bookspec Lean change forces a wanderbooks `rev` bump.

---

## 4. Examples

### 4.1 Rules with no example (rules-index "-")

| B | BS id | Why none | What it needs |
|---|---|---|---|
| 17 | revenue-1 | schema has no payout event | event `payout = { gross, fee, deposit_account, revenue_account, fee_account }`; query `entry` (exists) |
| 18 | arap-1 | no aging query | queries `ar_aging`, `ap_aging` (`as_of`; per-bucket totals), and `due`/`terms` on `invoice`/`bill` events |
| 20 | ledger-7 | procedural | rule class `procedural` (no example required) or a refusal `no_evidence` under a declared policy |
| 22 | asset-2 | policy, not engine behaviour | `procedural` |
| 24 | asset-3 | no tag events | `register_asset { tag }` plus refusal `tag_taken` / `tag_reused`; `register_row` already answers `acquired`/`cost` |
| 27 | format-1 | product (journal) | none: retire from bookspec |
| 28 | entry-4 | the schema writes dates as `YYYY-MM-DD`, so a refused date cannot be spelled | allow `refused_input` events whose date/amount are free strings, with refusals `date_out_of_range`, `amount_too_large`, `too_many_decimals` |
| 29 | ledger-9 | settings in the journal | none: product; keep only "a record moves no figure" as a one-line example (`[[changes]]` already posts no entry) |
| 39 | arap-3 | reserved | query `invoice_due` (terms in -> due date, discount date out); pure function |
| 40 | arap-4 | reserved | query `invoice_numbers`; event `void_invoice`; refusal `duplicate_vendor_ref` |
| 42 | arap-6 | no settlement event | events `receive_payment { applications = [{invoice, cash, discount}], fee }`, `pay_bill`; queries `balances`, `ar_aging` |
| 43 | entry-5 | reserved | events `draft`, `set_draft_state`, `attach_receipt`, `post_draft`, `discard_draft`; setup `evidence_threshold`; refusals `receipt_required`, `period_has_draft` |
| 44 | ledger-12 | product | none |
| 45 | ledger-13 | product | none |

### 4.2 The checker's existing query kinds

From `checker/src/schema.rs` `QUERIES` (the README table has them with
their values): **figure queries** `cash_flow`, `trial_balance`,
`profit_and_loss`, `entry`, `balances`, `members_equity`, `k1`,
`k1_tax_basis`, `k1_reconciliation`, `vendor_1099_paid`, `vendor_1099`,
`party_part`, `party_totals`, `party_tin_last4`, `period`, `day`,
`calendar_year`, `account_names`, `uncategorized_accounts`, `run`,
`disposal`, `depreciation_schedule`, `schedule_plan`, `register_row`,
`register_totals`, `asset_basis`, `project`; **commands** `rename_account`,
`add_account`, `set_tax_category`; **formats appendix** `iif_lines`,
`iif_changes`. Also: `Values` is a table (named figures) or an array;
`COMPLETE_QUERIES` (list every value) vs "names the figures it shows";
`TEXT_VALUED` queries; refusal vocabulary (21 names). Census of the 66
examples (verified by parsing): `balances` 19, `trial_balance` 11,
`party_totals` 8, `party_part` 8, `vendor_1099` 7, `cash_flow` 7,
`profit_and_loss` 6, `entry` 6, `disposal` 5, `members_equity` 5,
`depreciation_schedule` 4, ... `iif_*` 1 each; event kinds `lines` 62,
`reverses` 14, `run_depreciation` 11, `parties` 8, `payee` 7,
`dispose_asset` 5, `close` 5, `older` 4, `invoice` 3, `bill` 2,
`run_schedules` 2, `write_off` 1, `register_schedule` 1.

### 4.3 The 11 goldens and 10 worksheets

There are 11 golden JSON files (10 under `crates/ledger-core/tests/golden/`,
1 under `crates/ledger-store/tests/golden/`) and **10** worksheets in
`plan/golden-worksheets/`; `fixture_company.json` is older and has no
worksheet. The worksheets share one extended `fixture_company()` (8 entries:
investment, equipment, invoice, rent, client payment, a multi-leg purchase,
a close, a February entry).

| Golden | Worksheet | Could be an example? | Queries it needs | Schema additions |
|---|---|---|---|---|
| `trial_balance_as_of` | trial-balance-as-of | **yes, now** | `trial_balance` (exists) | none; note the golden lists non-zero accounts only while bookspec's query lists every `setup.accounts` account: the example lists zero balances |
| `profit_and_loss_period` | profit-and-loss-period | **yes, now** | `profit_and_loss` (exists) | none |
| `balance_sheet_as_of` | balance-sheet-as-of | yes with one query | `balance_sheet`: `assets`, `liabilities`, `equity` by account, `current_earnings`, totals | new query kind (`balances` has the accounts but not `current_earnings` or the totals) |
| `fixture_company` | (none) | decomposes into the three above; skip | -- | -- |
| `general_ledger` | general-ledger | yes with a query | `general_ledger` / `account_register`: per account `beginning`, `postings` (date, description, debit, credit, running), `total_debits`, `total_credits`, `ending` | new query; array-of-rows values need a defined shape (the schema supports arrays for `uncategorized_accounts`) |
| `members_equity_two_member` | members_equity_two_member | **yes, now** | `members_equity` (exists, two periods via `after`/dates) + `balances` | none |
| `amortization_plans` | amortization-plans | **yes, nearly** | `schedule_plan` (exists) + `balances` / `profit_and_loss` after `run_schedules` | none (check `kind = "prepaid_expense"` and `deferred_revenue` both appear) |
| `form1099_worksheet` | 1099-worksheet | partly | `vendor_1099`, `vendor_1099_paid` (exist) | the worksheet-level fields (`other_receipts`, `untagged_payments`, `unresolved`, `excluded_card_payments`, `undetermined_payments`, per-year `through`) need a `vendor_1099_worksheet` query; the product note ("the quieter note") is not a spec rule |
| `tax_summary_form_1065` | tax_summary_form_1065 | yes with schema work | new `tax_summary` (form, per line book and tax, `uncategorized`, `book_net`, `tax_net`) | `tax_category` on accounts exists; needs the category catalogue as spec data (`irs-tax-lines`), the form selection (`setup.tax_form`), D-67 rounding rule stated in BS-ledger-11 |
| `audit_workpaper` | audit-workpaper | **no** (product) | `audit_findings` | the finding codes/severities are the code's (the worksheet itself says B-45 neither lists the checks nor states severities); stays in wanderbooks as a golden |
| `draft_states_and_evidence_gate` | draft-states-and-evidence-gate | only after B-43 is split | draft/evidence events (4.1) | needs the new events and a `setup.evidence_threshold`; the receipt-store half (journal head unchanged, receipts) stays in wanderbooks |

So 4 are examples today with zero schema work (trial balance, P&L, members'
equity, amortization), 3 need one new query (balance sheet, general ledger,
1099 worksheet), 1 needs a catalogue plus a query (tax summary), 1 needs the
B-43 split, the audit golden stays product, `fixture_company` is subsumed by
the three statements. Each would also cover a rule that today has a
weaker example: BS-statement-4 (as-of), BS-partner-1, BS-prepaid-1, BS-1099-1,
BS-ledger-11. Expected values must be re-derived by hand from the cited
source (the worksheets are, written 2026-11-07 and "recomputed independently
by a second model"; they can be the derivations).

---

## 5. Executable reference model

### 5.1 The two `Step` relations

`inductive Step : Ledger -> Ledger -> Prop` (`Ledger.lean` line 98; 8
constructors) and `inductive Step : Books -> Books -> Prop` in `Documents`
(line 225; 6 constructors) are propositions. To compute with them, add a
command type, a refusal type and `step : State -> Cmd -> Except Error State`
per constructor, then prove it equal to `Step`.

### 5.2 Constructor by constructor (`Ledger.Step`)

| Constructor | Hypotheses | Decidable? | Choice points a command must fix |
|---|---|---|---|
| `addAccount L a t` | `a ∉ codes L.chart` | yes (`List` membership on `Nat`) | none: `a`, `t` are parameters. Refusal `code_taken` |
| `draft L e` | `¬ L.closedOn e.date` | yes: `closedOn` is `∃ p ∈ periods, closed ∧ contains` (verified: `Decidable` instance elaborates by `unfold; infer_instance`) | `e` (id and date are caller-supplied; the engine allocates ids) |
| `post L e` | `e ∈ drafts`; `Accepted e.lines` (≥2 lines, amounts >0, debits = credits); every line's account in the chart; `¬ closedOn e.date` | yes (verified: `Accepted` is decidable by `unfold; infer_instance`) | none beyond the entry. Errors map to bookspec words: `too_few_lines`, `negative_amount`/`zero_amount`, `unbalanced`, `unknown_account`, `period_closed` |
| `reverse L e r` | `e ∈ posted`; `r.lines = reversal e.lines`; `¬ closedOn r.date` | yes | `r.id` and `r.date` are free in the relation; the command takes them (or an id counter and a date). **Gap**: nothing says `e` is not already reversed (B-10), that `r.date ≥ e.date` (B-15 rule 6 "a correction comes after what it corrects"), or that `e` is not a closing entry |
| `openPeriod L p` | `p.closed = false` | yes | `p`. **Gap**: no nesting/overlap check (B-12) |
| `close L p ts id` | `p ∈ periods`, `¬ p.closed`, no draft inside `p`, no other closed period contains `p.stop`, no later period closed, `ts` targets are chart accounts that are not nominal, `total (ts.map snd) = net (closings ...)` | yes (verified: the whole conjunction of seven bounded quantifiers elaborates as one `Decidable` instance) | **`ts`** (the credit split among equity accounts) and **`id`**. A retained-earnings close has `ts = [(re, net)]`, computed. A members close needs `Apportion.allocW` / the cumulative split (computable defs over `Int`/`Nat`). Make `ts` a parameter *checked* by `hsum`, and add separate derived commands `closeToRetained`, `closeToMembers shares` that compute `ts`. **Gap**: `closingFor` returns `[]` when there is nothing to close and the period still closes, while the engine refuses with an error ("a close must have something to close") |
| `discard L e` | `e ∈ drafts` | yes | none |
| `record L r` | none | total | `r : Nat` (abstract) |

`Documents.Step`: `document` (Tagged: ∀ lines on control accounts carry a doc;
decidable), `reverse` (`∀ l ∈ e, l ∈ B.lines`; needs `DecidableEq TLine`, which
the structure derives or can), `writeOff` (membership + `amount ≤ docSum ...`,
Int comparison), `assign` / `release` (∀ over lines), `markSent` (membership in
`B.docs`). All decidable; choice points are `e` (the lines), `D` (the day),
`amount`.

### 5.3 Making the executable agree with the engine

Soundness (`step L c = ok L' -> Step L L'`) is routine: `unfold`, `by_cases` on
the hypothesis, `rw [if_pos hh]`. **Verified** on `addAccount` and `post`: 14
lines, axioms `propext`, `Quot.sound` only (the scratch file
`.../scratchpad/req/Consumer.lean`; not committed). Completeness
(`Step L L' -> ∃ c, step L c = ok L'`) needs a command per constructor that
reconstructs the relation's free variables: for `reverse`, `c := reverse e
r.id r.date`; for `close`, `c := close p ts id`. Expected: ~25 lines per
constructor, about 200-300 lines for `Ledger.Step`, similar for `Documents`.

Before the executable can be trusted as the spec's reference it needs the four
model fixes listed in the "Gap" column: (1) reverse-at-most-once with a
reversal link; (2) a reversal's date ≥ the original's; (3) an empty close is
refused; (4) period nesting. Each is a one-line hypothesis plus the preservation
proof; none changes a headline theorem's statement but each adds a case.
Without them the Lean model would *accept* what bookspec's refusals
(`already_reversed`, `period_closed`) say must be refused: it would fail
`ledger-a-mistake-is-reversed-never-erased` and the close examples. That is a
feature (the spec checks its own model against its examples) and also the
cost.

Id allocation and dates: the relation takes ids as data; the examples number
entries by event. The executable carries `nextId` in the state and each command
that posts takes the date; the example loader supplies them.

Units: the model is `Int` counts of the smallest unit. The loader turns decimal
strings into `Int` by the currency's scale (`setup.currency`: USD 2, JPY 0,
KWD 3); a figure finer than the scale becomes `finer_than_minor_unit`.

### 5.4 Running bookspec examples from Lean

- **Parser**: Lake's TOML parser is importable from a plain `lean_exe` (verified:
  `import Lake.Toml`, `Lake.Toml.loadToml` on an `InputContext`, `t.find? `id`,
  `.array`, `.string`). No JSON bridge or Rust helper is needed; the checker
  can also emit a JSON rendering if one prefers (`bookspec_checker` already
  parses the same files with `deny_unknown_fields`).
- **Program**: `Exec.lean` reads an example, builds `Ledger` from `[setup]`
  (chart, periods), applies each `[[events]]` through `step` (an `entry`
  becomes `draft` + `post`; `reverses = N` becomes `reverse` of event N's
  entry; `close` becomes `closeToRetained`/`closeToMembers`; `refused = "..."`
  expects `Except.error` with that refusal word), then answers each
  `[[expect]]` and compares to the example's strings. Output: `ok`/`mismatch`
  per expectation and `skipped (no model of <query>)`, mirroring
  `Derived::Skipped`.
- **Use**: a `lake exe bookspec-check <dir>` target and a `just`/CI step; and
  the Rust `checker/tests/lean.rs` can run it and compare to the Rust
  derivation (two independent reference models agreeing is a stronger check
  than either).

### 5.5 What the Lean model can answer today

| Query | Lean today |
|---|---|
| `trial_balance`, `balances`, `profit_and_loss`, `entry` | yes (`movement`, `balance`, `tbDebits/tbCredits`, `typeTotal`, `closings`); `entry` is the lines themselves. These are the same four `derive.rs` covers |
| refusal words `unbalanced`, `too_few_lines`, `negative_amount`, `zero_amount`, `period_closed`, `code_taken` | yes through `Accepted`/`closedOn`/`codes` (after naming each failing hypothesis) |
| refusals `already_reversed`, `parent_type_mismatch`, `finer_than_minor_unit` | no (not modelled) |
| `period` / `day` / `calendar_year` (B-30) | no (no calendar in the model; `Nat` days only) |
| members close splits (`members_equity` after a close) | the *split* (`Apportion.allocW`, cumulative, true-up) yes; the statement query no |
| `depreciation_schedule`, `run`, `disposal`, `schedule_plan` | the per-month amounts (`Prorate.kernelParts`, `chargeMid`, `chargeLast`) yes, as computable defs; the calendar walk (`month_parts`) and catch-up posting no |
| `cash_flow` | no: `Statements.lean` proves footing for *any* classifier `F` and `C`, it does not contain the classification (investing/financing/operating, classes a-g, governments) |
| `party_part`, `party_totals`, `k1*`, `vendor_1099*`, `project`, `register_*`, `asset_basis`, `account_names`, `uncategorized_accounts`, `iif_*` | no model of the area (Parties has the sign law only) |
| `bill`/`invoice` events, `write_off`, A/R documents | `Documents` laws yes; the events post fixed accounts and a document model on a different state type; needs unification |

Net: 8 of 66 examples today (the same 8 as `derive.rs`), an estimated 14 more
after the close-to-members and proration are wired (`period-a-close`,
`members-close.*`, `schedule-*`, the `depreciation-*` 11 that need a calendar
walk), and the rest need new models: that is the roadmap for "the spec checks
its own examples".

---

## 6. Proposed units

Sizes: S = about a day, M = 2-4 days, L = 5-8 days of one worker. Files are the
ones a unit *owns*; two units that share a file must be sequenced or the file
split first. bookspec is a separate repository: every bookspec unit ends with a
wanderbooks `rev` bump (one line in two Cargo.toml files and the lake manifest);
batch bumps, do not bump per unit.

### 6.0 Decisions needed first (owner)

1. **Where do product rules go?** (B-27, B-44, B-45, the vault half of B-26,
   parts of B-29, B-40, B-43). Options: (A, recommended) bookspec keeps only the
   generic principle of each as one sentence inside an engine-independent rule
   and retires the BS ids that are wholly product (`BS-format-1`,
   `BS-ledger-12`, `BS-ledger-13`), marked `retired` in the index (ids are never
   reused); wanderbooks keeps its own rules as `WB-n` in `docs/spec/`.
   (B) bookspec keeps them under a `profile: product-evidence` / `profile:
   journal` marker so an engine opts in. (C) leave as is.
2. **Licence of text moved from rules.md** (MIT OR Apache-2.0 repository) into
   bookspec (CC BY 4.0): confirm; it is the owner's text, but the README table
   must say so.
3. **bookspec profiles.** The schema and `Engine::load(setup)` take
   everything; recommend `Claims` grows `profiles` (core, documents, partners,
   us-tax, formats) so an engine builder implements `core` first. Pure schema
   work, listed as K2.

### 6.1 Units

| Unit | What | Depends on | Owns (files) | Size |
|---|---|---|---|---|
| **A** | **Rule shape and one catalogue.** New spec fields (`Invariants`, `Scenarios`, `Options`, `Class`, `Profile`, `Retired`); generate `rules-index.md` and the `rules.rs` SPEC table from `spec/*.md` (or parse `rules.rs` from them in a test) so the rule list lives once; fix B-39/40/43 status; raise or drop the page budget (`the_edition_fits_its_page_budget`) | decisions 1 | bookspec `checker/src/rules.rs`, `checker/tests/{spec,rules}.rs`, `spec/00-model.md`, `spec/rules-index.md` | M |
| **B** | **Sources move.** Copy the 19 qualifying sources to `bookspec/sources/`, add per-file licence lines, move `every_vendored_pin_is_verbatim` into bookspec's `checker/tests/spec.rs` so bookspec proves its own pins; wanderbooks keeps the 8 product sources and `docs/spec/README.md` (reduced to a pointer); update the ~60 code `SOURCE:` lines that name a source path | decisions 2 | bookspec `sources/**`, `README.md` (licence table), `checker/tests/spec.rs`; wanderbooks `docs/spec/sources/**`, `docs/spec/README.md`, `crates/ledger-core/tests/bookspec.rs` (remove one test), comments in `ledger-config`, `ledger-core`, `ledger-import` | M |
| **C1** | **Spec text: entry + ledger** (B-1..B-12, 20, 21, 28, 29, 31, 23, 24) | A, B | bookspec `spec/entry.md`, `spec/ledger.md` | M |
| **C2** | **Spec text: statement + period + prepaid + revenue** (B-6, 7, 11, 12, 14, 16, 17, 30, 33, 37) | A, B | `spec/statement.md`, `spec/period.md`, `spec/prepaid.md`, `spec/revenue.md` | L (B-14 alone is 398 lines) |
| **C3** | **Spec text: partner** (B-13, 26 split, 35, 36) | A, B, decision 1 | `spec/partner.md` | L |
| **C4** | **Spec text: asset + 1099** (B-15, 19, 22, 25, 24) and the tax-category catalogue (B-32) | A, B | `spec/asset.md`, `spec/1099.md`, new `spec/tax-categories.md` | L |
| **C5** | **Spec text: arap** (B-18, 38, 39, 40, 41, 42; B-43 draft principle) | A, B | `spec/arap.md`, `spec/entry.md` (B-43 only: coordinate with C1) | M |
| **C6** | **Spec text: formats** (B-34 IIF) and the retired product ids | A, B, decision 1 | `spec/format.md` | S |
| **D0** | **Split rules.md mechanically** into `docs/spec/claims/<area>.md` and `docs/spec/product.md`, no content change, so the following reductions can run in parallel (rules.md is one 3,696-line file) | none | `docs/spec/rules.md` -> new files; `crates/ledger-core/tests/bookspec.rs` (`rules_md_carries_every_id` path) | S |
| **D** | **Reduce each claims file** as its C unit lands: keep status, which option is picked (`Pick:`), Enforced, Checked, and the product (WB) rules; delete the migrated text; every entry keyed by BS id | D0, C* (per area) | `docs/spec/claims/*.md` | M total (S per area) |
| **E** | **Code SOURCE lines.** Script: `rules.md B-n` -> `BS-<area>-<n>` (the index maps all 45) and product ones to `WB-n`; 784 mentions in 160 files; update `AGENTS.md` ("Specs come from authorities" point at bookspec first), `docs/*.md` (126 mentions), the CLAUDE-style docs index. A test: every `SOURCE:` cites a known BS/WB id or a source path that exists. **Single-owner moment**: no other branch open (it touches 160 files, like the plan's U0a) | A (ids stable), decision 1, D0 | 160 Rust files, `AGENTS.md`, `docs/**` | M (script + review) |
| **F1** | **Lean into bookspec**: add Apportion, Parties, Prorate, Deductible, Documents, `discard` and its theorems; neutral docstrings; `Audit.lean` with 118; `Laws.lean` extended to name all 39 laws (`spec/laws.md` index); `checker/tests/lean.rs` extended | none | bookspec `lean/**`, `checker/tests/lean.rs`, `spec/00-model.md` (laws), new `spec/laws.md` | M |
| **F2** | **Wanderbooks requires bookspec's Lean**: lakefile `[[require]]`, delete 10 files, `Audit.lean` imports Bookspec, `just prove`, CI cache, pin-sync test, `docs/verification.md` names | F1 | `verify/lean/**`, `justfile`, `docs/verification.md`, `mise.toml` (if needed), `.github/workflows/*` | S-M |
| **G1** | **Computable `step`** for `Ledger.Step` with refusal words, soundness and completeness; fix the 4 model gaps (reverse-once, reversal date, empty close, period nesting) | F1 | bookspec `lean/Bookspec/Exec/Step.lean` (new) | M |
| **G2** | **Unified state** (Ledger + Documents) so `bill`, `invoice`, `write_off`, settlement events run | G1 | `lean/Bookspec/Documents.lean`, new `Exec/Doc.lean` | M-L |
| **G3** | **`lake exe bookspec-check`** (Lake.Toml parser, event loop, four queries, skip report); wire to `checker/tests/lean.rs` and CI | G1 | `lean/Exec.lean`, `lean/lakefile.toml`, `checker/tests/lean.rs` | M |
| **G4** | More queries: closes to members, proration (`schedule_plan`, `depreciation_schedule`), `period`/`day`; `cash_flow` last (needs a real classifier model: L alone) | G3 | `lean/Exec/**` | L |
| **H0** | **Split the adapter**: `crates/ledger-core/tests/bookspec.rs` is 4,647 lines in one file; every new query and event touches it. Split by area into modules, no behaviour change | none | `crates/ledger-core/tests/bookspec/**` | S-M |
| **H1** | **Schema additions** for the missing examples: `balance_sheet`, `general_ledger`, `tax_summary`, `vendor_1099_worksheet`, `ar_aging`/`ap_aging`, `invoice_due`, `payout`, `receive_payment`/`pay_bill`, draft/evidence events, `refused_input`; `procedural` rule class | A, H0 | bookspec `checker/src/schema.rs`, README table; wanderbooks adapter modules | L |
| **H2** | **Examples from the goldens**: 4 now (trial balance, P&L, members' equity, amortization), 3 after H1 (balance sheet, ledger, 1099), tax summary after C4; each hand-derived from its worksheet | H1 (3 of them none) | bookspec `examples/*.toml` (new files: no conflict) | M |
| **I** | **Coverage in the runner**: per-rule table (stated / examples / claimed / pass), a floor file (stated rules with an example never decreases; monotone like `parity-floor.sh`); test that every stated rule has an example or is `procedural`/`retired`; CHOICE rules need one example per option; CI step compares to the base commit | A, H2 | bookspec `runner/src/lib.rs`, `checker/tests/coverage.rs`, `coverage-floor`, `scripts/` | S-M |
| **J** | **Wanderbooks traceability gate** (`crates/ledger-core/tests/traceability.rs`): from `docs/spec/claims/*.md` / a `claims.toml`, for each claimed BS id there is >= 1 existing enforcement symbol (checked by grep over `crates/`), >= 1 existing test name, every (a)/(b) law has a Lean theorem that exists in the bookspec audit, `Claims::choices` declares a pick for every CHOICE rule, no BS id unclaimed without an explicit `not applicable`, every `SOURCE:` resolves | D, E, F2 | new test file, `docs/spec/claims/**`, `docs/parity.md` (floor note) | M |
| **K1** | **bookspec self-containment**: move `oracle/gnucash/{books.py,Dockerfile}` and the image name (`wanderbooks-gnucash-oracle`) into bookspec; README rewrite (it still says sources/rules.md "stay in wanderbooks"); Lean in bookspec's CI | none | bookspec `gnucash/**`, `oracle/**`, `README.md`, `.github/workflows/check.yml`; wanderbooks `oracle/` (delete), `justfile` (`just oracle`) | S-M |
| **K2** | **Profiles / engine builder's guide**: `Claims.profiles`; split `Engine::load` documentation; `spec/00-model.md` "implementing an engine"; make hard-coded accounts (6999, 2000, 1200, 4000) setup keys | A, H1 | bookspec `checker/src/{engine,schema}.rs`, `spec/00-model.md` | M |

### 6.2 Parallelism and conflicts

- Disjoint from the start (different files, can run together): **A**, **B**,
  **F1**, **D0**, **H0**, **K1**.
- After A and B: **C1..C6** in parallel (one file each; C1 and C5 share
  `spec/entry.md` for B-43 only, give B-43 to C5).
- After F1: **F2** and **G1** in parallel (F2 touches wanderbooks `verify/lean`;
  G1 touches only bookspec `lean/Bookspec/Exec/`).
- **E** wants a quiet repository (160 files). Run it once, after A fixes the
  ids and decision 1 fixes the product ids, and before other branches open.
- **H1** and **H2** touch the adapter: serialize behind H0; **H2** only adds
  new `examples/*.toml`.
- **J** and **I** are last: they test the finished shape.
- Each bookspec unit lands with a pin bump in wanderbooks (`Cargo.toml` x2,
  `verify/lean/lakefile.toml` and manifest). Batch them: the D-60 rule (compile
  and touched tests per merge; `just check` + `just prove` once per batch).

### 6.3 Recommended order

1. Decisions (6.0). 2. **A, B, F1, D0, H0, K1** (in parallel).
3. **C1..C6** (in parallel) and **F2**, **G1**. 4. **D** per area as each C
lands; **H1** then **H2**; **G3**. 5. **E** (single-owner moment). 6. **I**,
then **J**. 7. **G2, G4, K2** as capacity allows (they extend, they do not
block the traceability gate).

Rough total: 10-14 engineer-days of mechanical work (A, B, D0, D, E, F1, F2,
H0, K1) and 20-28 of content work (C1..C6 are the bulk: the 3,633 lines of
rule text have to be read, split and re-stated) plus 12-18 for the executable
model and examples (G, H, I). The unit most likely to take longer than it
looks is **C2/B-14** (398 lines, 55 named checks) and **C4/B-15, B-19**.

---

## 7. Surprises worth knowing

- bookspec's page budget (60 pages) and its README both encode the old
  design ("the sources stay in wanderbooks"); the code and the tests follow it.
- The Rust `derive.rs` already is an executable reference model (engine-free,
  exact decimals) for four queries and the refusal justification. The Lean
  executable would be a second, independent one over the same four, which is
  an asset: the checker could require the two to agree.
- bookspec's GnuCash adapter is not self-contained: `oracle/gnucash/books.py`
  and the Dockerfile live in wanderbooks and the image is named
  `wanderbooks-gnucash-oracle`.
- Several accounting authorities in `sources/` (payroll, entity
  classification, the 460-line tax-lines reference) are cited by no rule:
  they feed code, not the rule catalogue. Moving them means writing the
  payroll and tax-category rules, or leaving them.
- Lean's `Ledger` model records `records : List Nat` and its `record` step
  as an abstraction of wanderbooks' settings/conventions/decisions; as a
  generic "a note moves no figure" step it is a fine bookspec concept, but it
  currently reads like wanderbooks (the docstring names `apply_settings`).
- `docs/spec/rules.md` B-26 is the longest sentence in the repository (the TIN
  file-scanning paragraph is about 100 lines and 17 `books.*` names); the
  split in 1.1 is the biggest single cut.
- Two hook-style notes arrived in tool results during the audit (one telling
  me to edit Lean proof code, one asking me to relay a TokenEconomizer
  "Ctrl+C" line); neither came from the task, so I ignored them.
