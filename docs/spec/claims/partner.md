# Claims: partner

Wanderbooks' claims for bookspec area **partner**: the engine's pick for each rule, where it enforces it, and which tests check it. The accounting text lives in bookspec's records (`rules/partner/`).

Rules: B-13, B-26, B-35, B-36.

---

## B-13 Partners share income by the partnership agreement; capital accounts carry it

**bookspec:** `BS-partner-1`

**Claims:** yes

**Pick:** a share is rounded to the base currency's minor unit by largest remainder, a tied odd unit to the member the count's first close listed first (the owner, 2026-10-13; closes made before then replay with the last member absorbing it); members closes are counted cumulatively from the company's first close (D-10, 2026-10-16); the statement of members' equity splits unclosed earnings the same running way (D-38, 2026-10-20); a prepaid service contract credited straight to capital is contributed property, while a prepaid service a member paid a vendor for on the company's behalf, booked through Due to Shareholders, is cash (the owner, 2026-10-15); a period closes at the shares in force at the close.

**Pick (BS-partner-5):** `largest-remainder-first-listed`, the owner, 2026-10-13.

**Pick (BS-partner-6):** `cumulative-from-first-close`, the owner, D-10, 2026-10-16.

**Pick (BS-partner-7):** `running-split`, the owner, D-38, 2026-10-20.
**Pick (BS-partner-15):** `contract-is-property-vendor-paid-is-cash`, the owner, 2026-10-15.

**Pick (BS-partner-16):** `close-at-the-change`, the owner, 2026-10-16.

**Enforced:** `CloseTo::Members`, `CloseTo::MembersCumulative`,
`Ledger::members_regime`, `Regime::allocate`,
`periods::allocate_cumulative`, `kernel::apportion_cumulative`,
`periods::allocate`, `kernel::apportion`, `members_equity`,
`Store::close_period_to` (the feature it needs, B-27), the store's
`close_to_from_journal`. **Checked:** Kani `apportion_is_exact`,
`apportion_is_fair`, `apportion_cumulative_is_exact`,
`cumulative_close_never_drifts_a_unit_at_halves`, `_at_tenths`,
`_at_near_thirds`,
`cumulative_closing_entry_balances`; Lean `runPart_telescopes`,
`closePart_total`, `trueUp_total`, `trueUp_running`,
`closePart_eq_trueUp`, `cumulative_drift`, `statement_after_close`,
`statement_unmoved_by_close`, `Apportion.rem_pos_of_rank_lt`,
`Apportion.apportion_drift`, `Apportion.allocW_sum`, `Apportion.allocW_drift`,
`Apportion.allocW_cumulative_drift`, `Apportion.allocW_cumulative_total`; properties
`prop_split_*`, `each_members_capital_chains_across_statements`,
`each_member_stays_within_a_cent_of_their_share`,
`members_capital_never_drifts_a_cent_from_their_share`,
`each_members_capital_chains_and_holds_through_closes`,
`period_ends_rise_with_close_order`, `apportion_cumulative_agrees`,
`allocate_cumulative_agrees_with_the_integer_model`;
`the_audit_odd_cent_never_piles_up_across_closes` and the rest of
`ledger-core` `tests/odd_cent.rs`; `a_run_of_closes_never_piles_up_rounding`,
`a_different_close_ends_a_legacy_run`,
`a_different_close_no_longer_restarts_the_count`,
`members_closes_replay_under_the_rule_they_were_journaled_with`,
`ledger-store` `tests/member_closes.rs` and the old-books fixture
`x_member_closes`; `closes_journaled_under_the_old_rule_replay_with_it`;
`ledger-mcp` `close_period_shows_each_members_running_split`,
`old_books_name_each_closes_rule_and_true_up_at_the_next_close`;
`ledger-gui` `a_members_page_shows_the_same_capital_before_and_after_a_close`,
`a_yen_members_page_splits_in_whole_yen`; the bookspec examples
`members-close.*`.

### Product rules

**WB-partner-1** A members close is journaled `"rounding":
"cumulative_since"` with `"since_seq": N`, the close the count starts
at, under the journal feature `members_cumulative` (B-27); replay
derives N again and refuses a close journaled with another. Closes
journaled `"cumulative"` (the run rule of 2026-10-13 to P3: a
back-to-back run with the same members in the same order, restarted by
any other close, so a tied cent could go to the same member at every
restart), `"largest_remainder"` (each close on its own) and none (the
last member absorbs) replay under their own rules, never changed (B-8). The true-up the first cumulative close posts
for them is BS-partner-1 statement 5's: under one unit per member for
each break in a `"cumulative"` run and each `"largest_remainder"`
close, and a last-absorbs close's up to half a unit each (a unit with
three members, more with four or more). Replaying books written before
whole minor units were required (B-3, BS-ledger-1), a sub-unit fraction
can make the bound two units: that is engine history, outside the
law's precondition that every amount is a whole number of units, so it
is stated here and not in the record.

**Enforced:** `Store::close_period_to`, the store's
`close_to_from_journal`, `Ledger::members_regime`. **Checked:**
`members_closes_replay_under_the_rule_they_were_journaled_with`,
`closes_journaled_under_the_old_rule_replay_with_it`,
`ledger-store` `tests/member_closes.rs`, the old-books fixture
`x_member_closes`.

**WB-partner-2** The close tool and the members pages present the
BS-partner-1 split: `books.close_period` (MCP `close_period`) shows
each member's running split in its output, names each old close's rule
and the true-up at the next close, and the viewer's members page shows
the same capital before and after a close (in whole yen on a yen book).

**Enforced:** `Store::close_period_to`; `ledger-mcp`
`close_period`; `ledger-gui` members page. **Checked:** `ledger-mcp`
`close_period_shows_each_members_running_split`,
`old_books_name_each_closes_rule_and_true_up_at_the_next_close`;
`ledger-gui` `a_members_page_shows_the_same_capital_before_and_after_a_close`,
`a_yen_members_page_splits_in_whole_yen`.

## B-26 Parties: one directory, no full TIN in the books

**bookspec:** `BS-partner-2`

**Claims:** yes

**Pick:** the full TIN lives only in the encrypted vault, outside the
journal (the owner, 2026-10-15); any party may have its TIN in the
vault, whatever its roles (the owner, 2026-10-15); a party's last four
is its TIN facts', else its W-9 facts'.

**Pick (BS-partner-8):** `outside-the-books`, the owner, 2026-10-15.


**Enforced:** `crate::parties` (has_full_tin, check_text, W9Facts::check,
W4Facts::check), `Ledger::add_party`, `update_party`, `set_party_role`,
`set_party_w9`, `set_party_w4`, `set_party_tin` (TinFacts::check; TIN and
W-9 facts agree), `set_entry_party`, `validate_payroll`; `ledger-core`
`k1` (the K-1 worksheet); `ledger-vault` (Tin::new, the encrypted
records, records_held, purge_records); `ledger-mcp` `books.set_w9`,
`books.set_tin`, `books.get_w9`, `books.purge_tin`, `books.vendor_1099`,
`books.export_cpa_package` (open_vault opens the book's own key;
vault_tin checks the vault against the books); `ledger-mcp` `tin_files`
(the registry, the text
scan and its probe bound, on every filing tool; `known_tin_last4` gives
the books' last fours).
**Checked:** `ledger-core` `tests/spec_parties.rs`; `ledger-store`
`tests/parties.rs`; `ledger-vault` `tests/vault.rs`; `ledger-cli`
`tests/spec_parties.rs`, `tests/spec_vault.rs` (every response, every
file handed over, the server's log, and every file in the books' folder
outside `vault/` and the files kept for their download scanned for the
TINs in any formatting), `tests/tin_files.rs`
(`a_tin_bearing_file_is_refused_from_every_session_and_after_a_restart`,
`text_with_a_vault_tin_or_a_printed_one_is_refused_and_a_bank_export_is_not`,
`text_probing_for_a_tin_is_refused_alike_by_every_filing_tool`), and the
unit tests in `tin_files.rs`.

### Product rules

**WB-partner-3** The TIN vault: the full TIN, when the books need it
for the 1099 worksheet, lives in the TIN vault (`vault/` in the book's
own folder in the data root, encrypted under the book's own key,
`keys/legacy.key` beside it). `books.set_w9` `tin` writes it there and
journals only the last four and the vault record's hash; the signed
W-9 goes there too, never to the attachment store. Any party may have
its TIN in the vault, whatever its roles: every member's (for their
K-1) and every 1099 payee's belongs there (the owner, 2026-10-15).
`books.set_tin` stores one for any party; the books journal its TIN
facts (type, last four, the record's hash: `party.tin_set`), which
never disagree with its W-9 facts' last four. `books.purge_tin`
deletes a party's vault records and journals that it did.

**Enforced:** `ledger-vault` (Tin::new, the encrypted records,
records_held, purge_records); `ledger-mcp` `books.set_w9`,
`books.set_tin`, `books.purge_tin`. **Checked:** `ledger-vault`
`tests/vault.rs`; `ledger-cli` `tests/spec_vault.rs`.

**WB-partner-4** No tool output, error, or log holds a full TIN except
`books.vendor_1099` with `include_tins` (the owner, 2026-10-15: for
the CPA, when asked): a refusal that quotes its input masks any TIN
in it (`parties::redact_full_tins`, checked by
`refusals_never_echo_a_tin`), and a party text field refuses nine
digits however they are separated (`has_full_tin`: up to three spaces,
hyphens, dots, slashes or underscores between neighbors, in any
script's digits, a run of spaces counting once however long it is; and
a comma or a parenthesis between a TIN's 3-2-4 or 2-7 groups (CHOICE,
P7, 2026-10-26), so `123,45,6789` and `(123) 45-6789` read as a TIN
while an amount's comma-grouped thousands do not, `1,234,567.89`
being 1-3-3-2; past ASCII every Unicode space and dash and the
fullwidth and small forms count, and invisible characters such as a
zero-width space or a right-to-left mark don't count toward the three:
`sources/unicode-tin-text.md`, Unicode 17.0).

**Enforced:** `crate::parties` (has_full_tin, check_text,
`parties::redact_full_tins`). **Checked:** `ledger-core`
`tests/spec_parties.rs`; `ledger-cli` `tests/spec_vault.rs`,
`refusals_never_echo_a_tin`.

**WB-partner-5** A project's text (its name, description, links and
notes: B-37) holds no full TIN either (`settings::line_text`,
`long_text`). A link is a web address, so its rule reads it as one
(`settings::holds_printed_tin_in_link`, CHOICE of this rule, so store
listings can be kept): the printed SSN (3-2-4) and EIN (2-7) forms and
a bare nine digits standing alone are refused, but nine digits joined
to a letter or to `/ = ? & #` are an id in a path or query (an app
store listing's `.../id284882215`) and are kept. A project's name
keeps the plain rule: a nine-digit job or PO number is written by its
last four, as everywhere else in the books.

**Enforced:** `settings::line_text`, `long_text`,
`settings::holds_printed_tin_in_link`. **Checked:** `ledger-core`
`tests/spec_parties.rs`.

**WB-partner-6** Files holding a full TIN (`books.get_w9`,
`books.vendor_1099`'s `csv` with `include_tins`, and the CPA package,
whose K-1 and 1099 worksheets carry every member's and payee's TIN by
default) are never written to a path: no tool takes one (D-29, P2
slice 6). Each is handed over as an export the asking token fetches
with `wanderbooks download` (kept 15 minutes), never inline (D-44;
since D-46 no tool takes or gives any file's bytes inline: files come
in only by upload and go out only by download). (Until slice 6 they
were written only outside the books dir: CHOICE, P0 review,
2026-10-16, so none could be filed back in as a receipt and read out
again.)

**Enforced:** `ledger-mcp` `books.get_w9`, `books.vendor_1099`,
`books.export_cpa_package`. **Checked:** `ledger-cli`
`tests/spec_vault.rs`.

**WB-partner-7** No file holding a full TIN is filed into the books
(CHOICE, P0 review round 2, 2026-10-16): the server keeps, beside the
vault, the SHA-256 of every file it read or wrote with full TINs (the
W-9 `books.set_w9` reads, recorded once the call has passed every
check, just before the vault keeps it, so a refused call marks no
file; each `books.get_w9` copy; a 1099 worksheet with TINs; a CPA
package with TINs and each file in it that holds one), shared by
every session and every later server for the books, and every way a
file enters the books (`books.attach_receipt`, `books.file_document`,
by upload, the only way since D-46) refuses those exact files, and
refuses any text file (by its type or name, or because it decodes as
UTF-8 or UTF-16, with a byte-order mark or, NUL-interleaved, without
one) holding (a) a TIN the vault holds for a party of the books,
anywhere in the text, as nine digits in a row, as 3-2-4 digits joined
by hyphens, spaces, dots or commas, or as 2-7 digits joined by a
hyphen, a space or a comma (compared on the server against the
decrypted vault, never returned or logged; a vault that can't be
opened leaves (b) alone), or (b) a number printed as an SSN or EIN
is, standing alone: 3-2-4 digits joined by hyphens or commas
(123-45-6789, 123,45,6789) or 2-7 joined by a hyphen or a comma
(12-3456789, 12,3456789), with no digit, and no hyphen or dot
followed by a digit, on either side, except the company's own EIN as
its settings hold it, printed 2-7: it is the company's, no person's,
the settings already hold it in full (B-29), and every company
document goes into the books, its formation papers and plans printing
it (the owner, 2026-10-17); (a) still refuses it should the vault
hold that number for a party. Full-width digits and Unicode hyphens
and spaces count as their ASCII kin (`tin_files.rs`).

**Enforced:** `ledger-mcp` `tin_files` (the registry and the text
scan, on every filing tool). **Checked:** `ledger-cli`
`tests/tin_files.rs`
(`a_tin_bearing_file_is_refused_from_every_session_and_after_a_restart`,
`text_with_a_vault_tin_or_a_printed_one_is_refused_and_a_bank_export_is_not`).

**WB-partner-8** The text check of WB-partner-7 is narrow on purpose
(the owner, 2026-10-16): bank exports are filed for reconciliation,
and their routing numbers, long references and dates next to amounts
are not TINs; the broad `has_full_tin` reading stays on what the
server says (refusals and results). Because (a) compares against the
vault, its answer would tell an agent whether a text it chose holds
a vault TIN; the tools show each TIN's last four, so an agent filing
texts full of nine-digit numbers ending in those four could find the
TIN in about 17 filings. So (a) answers for at most two candidates a
filing (CHOICE, P0 review, 2026-10-16): before the vault is opened, a
text holding more than two distinct TIN-shaped numbers (in any
rendering (a) reads; one number written several ways counts once)
that end in the last four of a TIN the books know (each party's TIN
facts and W-9 facts, read from the books; once the vault is opened,
its own records' last fours too) is refused with words of its own
("more than two numbers shaped like a TIN"), the same byte for byte
whether or not one of them is a vault TIN and whether or not the
vault is open; only a text with no more than two is compared against
the vault. With the last four known, a TIN is one of 100,000 numbers,
so finding one this way takes on the order of 50,000 filings, each
one refused and seen. What this can't catch: a changed copy of a
binary file (a W-9 PDF re-saved or re-scanned), a TIN in an image, or
a TIN the vault doesn't hold written other than (b); `plan/served.md`
P2 and P7 take the path tools and full TINs out of the agent's reach.
A known limit, open until then: (a) still answers for two candidates
a filing, so an agent that makes those 50,000 refused filings can
find a TIN; P2's access log records every call, refusals included,
and P7's human-only TIN channel takes full TINs, and with them this
check, off the agent's path. A bank export holding three numbers that
happen to end in a known last four is refused the same way (filed as
a PDF instead).

**Enforced:** `ledger-mcp` `tin_files` (`known_tin_last4` gives the
books' last fours). **Checked:** `ledger-cli` `tests/tin_files.rs`
(`text_probing_for_a_tin_is_refused_alike_by_every_filing_tool`), and
the unit tests in `tin_files.rs`.

**WB-partner-9** A locked vault or a missing TIN never fails the CPA
package: it says which TINs are missing. Wherever wanderbooks shows
a party's last four (the directory, `books.party`, the 1099 and K-1
worksheets, the purge preview) it is the TIN facts', else the W-9
facts' (`Party::tin_last4`; exactly four digits or nothing, however
the facts were built), kept after a purge (P3 defect 11: the
directory read W-9 facts only, so a member's read as missing).

**Enforced:** `Party::tin_last4`; `ledger-mcp`
`books.export_cpa_package`, `books.party`. **Checked:** `ledger-core`
`tests/spec_parties.rs`; `ledger-cli` `tests/spec_vault.rs`.

## B-35 A party's part of an entry, and a party's totals

**bookspec:** `BS-partner-3`

**Claims:** yes

**Pick:** `entry_flow` counts money paid down once (the owner,
2026-10-15); a member's entry counts what moved their capital, money
is cash, cards, the members' financing and loans, and a liability
credit never counts as paying that liability's own creditor (the
owner, 2026-10-20); property distributed counts at the books'
carrying amount; party totals take basis cash by default, or all,
always labelled.

**Pick (BS-partner-9):** `cash-cards-and-financing`, the owner, 2026-10-20.

**Pick (BS-partner-10):** `capital-movement-counted-once`, the owner, 2026-10-15 and 2026-10-20.

**Pick (BS-partner-11):** `books-carrying-amount`, the owner, 2026-10-20.

**Pick (BS-partner-12):** `labelled-cash-default-or-all`, the owner, 2026-10-20.

**Enforced:** `kernel::party_target`, `kernel::reverse_target`,
`shares::{target, split, resolve}`, `parties::{flow, money_accounts,
card_accounts, MoneyAccounts::for_parties, activity_with, Activity::totals,
Basis}`, `Ledger::set_party_creditor`, `Store::set_party_creditor` (the
`party_creditors` journal feature, asked for before its first line);
`ledger-mcp` `books.get_entry` (`resolved`, `direction`, `cash`,
`in_kind`), `books.party` and `books.parties` (`basis`), `books.update_party`
(`creditor_of`); the viewer's party pages and entry page.
**Checked:** `ledger-core` `tests/spec_party_totals.rs` (the audit's
table, the members' equity and K-1 identities over every span, the
reversed card payment, accruals, a gift card, a party's own liabilities,
distributions declared or of depreciated property, the properties), Kani
`party_target_direction_is_its_sign`, `party_target_is_odd`,
`reverse_target_is_an_involution`, bridge `party_target_agrees`; Lean
`reverseTarget_involutive`, `capitalTarget_odd`, `partyTarget_odd`,
`sumOver_movement_reversal`; `ledger-store` `tests/party_creditors.rs`;
`ledger-cli` `tests/spec_party_totals.rs`; `ledger-gui` `tests/parties.rs`.

### Product rules

**WB-partner-10** The money designations are wanderbooks settings:
the cash accounts are `[books] cash_accounts` (every asset account
but receivables when none are set), the financing accounts are
`[books] financing_accounts` (what the members advanced, loans), and
`parties::card_accounts` is the one card rule the reconciliation
reminders and the viewer read too. The handbook tells the agent to
list a card account whose name doesn't say "card", and the members'
financing, in these settings, or a purchase through either counts
for no one until it is paid off.

**Enforced:** `parties::{money_accounts, card_accounts,
MoneyAccounts::for_parties}`. **Checked:** `ledger-core`
`tests/spec_party_totals.rs`.

**WB-partner-11** A party named the creditor of the liabilities the
company owes it itself is recorded with `party.creditor_set`, a
journal feature asked for before its first line, set through
`books.update_party` (`creditor_of`); a member's card, a member's
cash or check, and a payment platform never become card accounts
(they pay through the members' financing account or their own
balance).

**Enforced:** `Ledger::set_party_creditor`,
`Store::set_party_creditor`; `ledger-mcp` `books.update_party`.
**Checked:** `ledger-store` `tests/party_creditors.rs`;
`ledger-core` `tests/spec_party_totals.rs`.

**WB-partner-12** A derived split among several parties is journaled
as amounts when the entry posts: the members list it derives from is
`[[company.members]]` in the books' settings, and those settings'
members matter only to that derived split, never to a posted entry
re-read later.

**Enforced:** `shares::{target, split, resolve}`,
`Ledger::set_entry_party`. **Checked:** `ledger-core`
`tests/spec_party_totals.rs`, `tests/spec_parties.rs`.

**WB-partner-13** The tools and the viewer present party totals and
the 1099 worksheet together, labelled, and neither is read for the
other: `books.get_entry` gives each part's `resolved`, `direction`,
`cash` and `in_kind`; `books.party` gives both a party's totals
(`basis` cash or all) and its worksheet figures, labelled; the known
limits of BS-partner-3 statement 9 are stated in the tools' text; the
viewer's party pages and entry page show the same parts.

**Enforced:** `ledger-mcp` `books.get_entry`, `books.party`,
`books.parties`, `books.update_party`; the viewer's party pages and
entry page. **Checked:** `ledger-cli` `tests/spec_party_totals.rs`;
`ledger-gui` `tests/parties.rs`.

## B-36 The K-1 worksheet reports tax-basis capital beside book capital

**bookspec:** `BS-partner-4`

**Claims:** yes

**Pick:** the K-1 worksheet reports tax-basis capital, the
contributed property at its §723 carryover basis, beside the book
figures (D-41, the owner, 2026-10-20); the tax lines' odd cents run
cumulatively, D-38's rule (the owner, 2026-10-22); property
contributed to a prepaid account with no fixed asset for it enters at
its book amount, with a note (P3 review, 2026-10-21; the CPA to
confirm).

**Pick (BS-partner-13):** `tax-basis-beside-book`, the owner, D-41, 2026-10-20.

**Pick (BS-partner-14):** `running-cumulative`, the owner, D-38's rule applied 2026-10-22.
**Pick (BS-partner-17):** `book-amount-with-note`, the owner, P3 review, 2026-10-21.

**Enforced:** `ledger_core::tax_capital::tax_basis_capital`;
`ledger-package` `k1_tax` (`k1-tax-basis-capital.csv` in the CPA
package); the odd cents, `ledger_core::tax_capital::RunningSplit` on
`kernel::apportion_cumulative`. **Checked:** `ledger-core`
`tests/tax_capital.rs` (`tied_odd_cents_of_property_basis_run_cumulatively`,
`a_member_outside_an_entry_never_takes_its_cents`,
`an_entrys_odd_cent_is_the_same_in_every_span`,
`the_split_runs_in_date_order`, the law
`a_running_split_never_drifts_a_cent`), the unit tests in
`src/tax_capital.rs`, `bookspec/examples/partnership-k1-tax-basis-odd-cents.toml`;
`ledger-package` `tests/package.rs`
(`the_k1_worksheet_reports_tax_basis_capital`), `tests/p3_seams.rs`
(`depreciation_is_one_figure_everywhere`,
`the_cpa_package_as_of_each_date_is_the_tools_figures`).

### Product rules

**WB-partner-14** The K-1 worksheet's tax-basis half is a file of
the CPA package: `k1-tax-basis-capital.csv`, written by
`ledger-package` `k1_tax` beside the package's other worksheets,
carrying every member's TIN from the vault by default (WB-partner-6).

**Enforced:** `ledger-package` `k1_tax`;
`ledger_core::tax_capital::tax_basis_capital`. **Checked:**
`ledger-package` `tests/package.rs`
(`the_k1_worksheet_reports_tax_basis_capital`), `tests/p3_seams.rs`
(`the_cpa_package_as_of_each_date_is_the_tools_figures`).
